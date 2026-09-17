use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context, Result};

use super::AcceleratorKind;

const OPENVINO_RUNNER_ENV: &str = "TACHYON_OPENVINO_RUNNER";
const EDGETPU_RUNNER_ENV: &str = "TACHYON_EDGETPU_RUNNER";

#[derive(Clone, Debug)]
pub(crate) struct VendorAcceleratorRuntime {
    kind: AcceleratorKind,
    runner: PathBuf,
    model: PathBuf,
}

impl VendorAcceleratorRuntime {
    pub(crate) fn try_load(kind: AcceleratorKind, model: impl AsRef<Path>) -> Result<Option<Self>> {
        let Some((env_name, marker)) = runner_contract(kind) else {
            return Ok(None);
        };
        let Some(runner) = std::env::var_os(env_name).map(PathBuf::from) else {
            return Ok(None);
        };
        let model = model.as_ref();
        validate_model_format(kind, model)?;
        let output = Command::new(&runner)
            .arg("--probe")
            .output()
            .with_context(|| {
                format!(
                    "failed to start {} runner `{}`",
                    kind.as_str(),
                    runner.display()
                )
            })?;
        if !output.status.success() {
            bail!(
                "{} runner probe failed with status {}: {}",
                kind.as_str(),
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let probe = String::from_utf8_lossy(&output.stdout);
        if !probe.contains(marker) {
            bail!(
                "{} runner probe did not report required device marker `{marker}`",
                kind.as_str()
            );
        }
        Ok(Some(Self {
            kind,
            runner,
            model: model.to_path_buf(),
        }))
    }

    pub(crate) fn execute(&self, input: &[u8]) -> Result<Vec<u8>> {
        let output = Command::new(&self.runner)
            .arg("--model")
            .arg(&self.model)
            .arg("--input-hex")
            .arg(hex_encode(input))
            .output()
            .with_context(|| {
                format!(
                    "failed to execute {} model through `{}`",
                    self.kind.as_str(),
                    self.runner.display()
                )
            })?;
        if !output.status.success() {
            bail!(
                "{} inference failed with status {}: {}",
                self.kind.as_str(),
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(output.stdout)
    }
}

pub(crate) fn is_available(kind: AcceleratorKind) -> bool {
    let Some((env_name, marker)) = runner_contract(kind) else {
        return false;
    };
    let Some(runner) = std::env::var_os(env_name) else {
        return false;
    };
    Command::new(runner)
        .arg("--probe")
        .output()
        .is_ok_and(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).contains(marker)
        })
}

fn runner_contract(kind: AcceleratorKind) -> Option<(&'static str, &'static str)> {
    match kind {
        AcceleratorKind::Npu => Some((OPENVINO_RUNNER_ENV, "OPENVINO_NPU")),
        AcceleratorKind::Tpu => Some((EDGETPU_RUNNER_ENV, "EDGE_TPU")),
        _ => None,
    }
}

fn validate_model_format(kind: AcceleratorKind, model: &Path) -> Result<()> {
    let name = model
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match kind {
        AcceleratorKind::Npu if name.ends_with(".xml") || name.ends_with(".blob") => Ok(()),
        AcceleratorKind::Tpu if name.ends_with("_edgetpu.tflite") => Ok(()),
        AcceleratorKind::Npu => {
            bail!("OpenVINO NPU models must use `.xml` IR or compiled `.blob` format")
        }
        AcceleratorKind::Tpu => {
            bail!("Coral models must be Edge-TPU-compiled `*_edgetpu.tflite` files")
        }
        _ => bail!("vendor runtime only supports NPU or TPU"),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_formats_are_fail_closed() {
        assert!(validate_model_format(AcceleratorKind::Npu, Path::new("model.xml")).is_ok());
        assert!(validate_model_format(AcceleratorKind::Npu, Path::new("model.onnx")).is_err());
        assert!(
            validate_model_format(AcceleratorKind::Tpu, Path::new("model_edgetpu.tflite")).is_ok()
        );
        assert!(validate_model_format(AcceleratorKind::Tpu, Path::new("model.tflite")).is_err());
    }

    #[test]
    fn runner_input_is_hex_encoded_without_shell_interpolation() {
        assert_eq!(hex_encode(b"\0tachyon\xff"), "0074616368796f6eff");
    }
}
