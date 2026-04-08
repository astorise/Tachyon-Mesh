use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};
use wasi_nn::{ExecutionTarget, GraphBuilder, GraphEncoding, TensorType};

const DEFAULT_MODEL_DIR: &str = "/models";
const DEFAULT_MODEL_FILE: &str = "model.onnx";

#[derive(Debug, Deserialize)]
struct InferenceRequest {
    #[serde(default = "default_model")]
    model: String,
    shape: Vec<usize>,
    values: Vec<f32>,
    #[serde(default = "default_output_len")]
    output_len: usize,
    #[serde(default)]
    response_kind: ResponseKind,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ResponseKind {
    #[default]
    Floats,
    Text,
}

#[derive(Debug, PartialEq, Serialize)]
struct FloatInferenceResponse {
    model: String,
    output: Vec<f32>,
    output_bytes: usize,
}

#[derive(Debug, PartialEq, Serialize)]
struct TextInferenceResponse {
    model: String,
    text: String,
    output_bytes: usize,
}

fn default_model() -> String {
    DEFAULT_MODEL_FILE.to_owned()
}

fn default_output_len() -> usize {
    4
}

#[no_mangle]
pub extern "C" fn faas_entry() {
    let body = match read_request_body() {
        Ok(body) => body,
        Err(error) => {
            println!(
                "{}",
                json_error(&format!("failed to read request body: {error}"))
            );
            return;
        }
    };

    match handle_request(&body) {
        Ok(response) => println!("{response}"),
        Err(error) => println!("{}", json_error(&error)),
    }
}

fn read_request_body() -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    io::stdin().read_to_end(&mut body)?;
    Ok(body)
}

fn handle_request(body: &[u8]) -> Result<String, String> {
    let request = parse_request(body)?;
    ensure_tensor_matches_shape(&request)?;

    let graph = load_graph(&request.model)?;
    let mut execution = graph
        .init_execution_context()
        .map_err(|error| format!("failed to create execution context: {error}"))?;
    execution
        .set_input(0, TensorType::F32, &request.shape, &request.values)
        .map_err(|error| format!("failed to set input tensor: {error}"))?;
    execution
        .compute()
        .map_err(|error| format!("WASI-NN compute failed: {error}"))?;

    match request.response_kind {
        ResponseKind::Floats => {
            let mut output = vec![0f32; request.output_len];
            let output_bytes = execution
                .get_output(0, &mut output)
                .map_err(|error| format!("failed to read output tensor: {error}"))?;
            let output_len = output_bytes / std::mem::size_of::<f32>();
            if output_len > output.len() {
                return Err(format!(
                    "model produced {output_len} floats but only {} slots were allocated",
                    output.len()
                ));
            }
            output.truncate(output_len);

            serde_json::to_string(&FloatInferenceResponse {
                model: request.model,
                output,
                output_bytes,
            })
            .map_err(|error| format!("failed to serialize inference response: {error}"))
        }
        ResponseKind::Text => {
            let mut output = vec![0u8; request.output_len];
            let output_bytes = execution
                .get_output(0, &mut output)
                .map_err(|error| format!("failed to read output tensor: {error}"))?;
            if output_bytes > output.len() {
                return Err(format!(
                    "model produced {output_bytes} bytes but only {} slots were allocated",
                    output.len()
                ));
            }
            output.truncate(output_bytes);
            let text = String::from_utf8(output)
                .map_err(|error| format!("model produced non UTF-8 text output: {error}"))?;

            serde_json::to_string(&TextInferenceResponse {
                model: request.model,
                text,
                output_bytes,
            })
            .map_err(|error| format!("failed to serialize inference response: {error}"))
        }
    }
}

fn parse_request(body: &[u8]) -> Result<InferenceRequest, String> {
    serde_json::from_slice(body).map_err(|error| {
        format!(
            "request body must be JSON like {{\"shape\":[1,4],\"values\":[1.0,2.0,3.0,4.0],\"output_len\":4}}: {error}"
        )
    })
}

fn ensure_tensor_matches_shape(request: &InferenceRequest) -> Result<(), String> {
    if request.shape.is_empty() {
        return Err("`shape` must include at least one positive dimension".to_owned());
    }
    if request.shape.contains(&0) {
        return Err("`shape` dimensions must all be greater than zero".to_owned());
    }
    if request.output_len == 0 {
        return Err("`output_len` must be greater than zero".to_owned());
    }

    let expected_values = request
        .shape
        .iter()
        .try_fold(1usize, |accumulator, dimension| {
            accumulator.checked_mul(*dimension)
        })
        .ok_or_else(|| "`shape` is too large to validate safely".to_owned())?;

    if expected_values != request.values.len() {
        return Err(format!(
            "`shape` expects {expected_values} input values but request provided {}",
            request.values.len()
        ));
    }

    Ok(())
}

fn resolve_model_path(model: &str) -> Result<PathBuf, String> {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return Err("`model` must not be empty".to_owned());
    }

    let relative = Path::new(trimmed);
    if relative.is_absolute() {
        return Err("`model` must be relative to `/models`".to_owned());
    }
    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err("`model` must not escape the sealed `/models` directory".to_owned());
    }

    Ok(Path::new(DEFAULT_MODEL_DIR).join(relative))
}

fn load_graph(model: &str) -> Result<wasi_nn::Graph, String> {
    if should_use_preloaded_model(model) {
        return GraphBuilder::new(GraphEncoding::Onnx, ExecutionTarget::CPU)
            .build_from_cache(model)
            .map_err(|error| format!("failed to load preloaded model alias `{model}`: {error}"));
    }

    let model_path = resolve_model_path(model)?;
    let model_bytes = fs::read(&model_path).map_err(|error| {
        format!(
            "failed to read ONNX model `{}`: {error}",
            model_path.display()
        )
    })?;
    GraphBuilder::new(GraphEncoding::Onnx, ExecutionTarget::CPU)
        .build_from_bytes([model_bytes])
        .map_err(|error| {
            format!(
                "failed to load ONNX model `{}`: {error}",
                model_path.display()
            )
        })
}

fn should_use_preloaded_model(model: &str) -> bool {
    let trimmed = model.trim();
    !trimmed.is_empty()
        && !trimmed.contains('/')
        && !trimmed.contains('\\')
        && !trimmed.contains('.')
}

fn json_error(message: &str) -> String {
    serde_json::json!({ "error": message }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_request_accepts_valid_payload() {
        let request = parse_request(
            br#"{"model":"sum.onnx","shape":[1,4],"values":[1.0,2.0,3.0,4.0],"output_len":4}"#,
        )
        .expect("request should parse");

        assert_eq!(request.model, "sum.onnx");
        assert_eq!(request.shape, vec![1, 4]);
        assert_eq!(request.values.len(), 4);
        assert_eq!(request.output_len, 4);
        assert_eq!(request.response_kind, ResponseKind::Floats);
    }

    #[test]
    fn resolve_model_path_rejects_parent_segments() {
        let error = resolve_model_path("../secret.onnx").expect_err("parent directory must fail");

        assert!(error.contains("must not escape"));
    }

    #[test]
    fn ensure_tensor_matches_shape_rejects_mismatched_payloads() {
        let request = InferenceRequest {
            model: default_model(),
            shape: vec![1, 8],
            values: vec![1.0, 2.0, 3.0, 4.0],
            output_len: 4,
            response_kind: ResponseKind::Floats,
        };

        let error =
            ensure_tensor_matches_shape(&request).expect_err("mismatched tensor size must fail");

        assert!(error.contains("expects 8 input values"));
    }

    #[test]
    fn preloaded_model_aliases_skip_filesystem_resolution() {
        assert!(should_use_preloaded_model("llama3"));
        assert!(!should_use_preloaded_model("model.onnx"));
        assert!(!should_use_preloaded_model("nested/model.onnx"));
    }
}
