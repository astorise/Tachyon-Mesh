mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "faas-guest",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};

const DEFAULT_INDEX: &str = "tenant-kb";
const DEFAULT_EMBEDDING_MODEL: &str = "demo-embedding";
const DEFAULT_CHAT_MODEL: &str = "demo-chat";
const FALLBACK_EMBEDDING_DIM: u32 = 16;

struct Component;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RagRequest {
    query: String,
    #[serde(default = "default_index")]
    index: String,
    #[serde(default = "default_top_k")]
    top_k: u32,
    #[serde(default)]
    documents: Option<Vec<InputDocument>>,
    #[serde(default = "default_embedding_model")]
    embedding_model: String,
    #[serde(default = "default_chat_model")]
    chat_model: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct InputDocument {
    id: String,
    text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RagResponse {
    query: String,
    index: String,
    answer: String,
    matches: Vec<RagMatch>,
    embedding_source: String,
    completion_source: String,
}

#[derive(Debug, Serialize)]
struct RagMatch {
    id: String,
    score: f32,
    text: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingResponse {
    data: Vec<OpenAiEmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    content: String,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        match handle_rag(req.body) {
            Ok(body) => response(200, body),
            Err(error) => response(400, json_error(&error)),
        }
    }
}

fn handle_rag(body: Vec<u8>) -> Result<Vec<u8>, String> {
    let request: RagRequest =
        serde_json::from_slice(&body).map_err(|error| format!("invalid RAG request: {error}"))?;
    if request.query.trim().is_empty() {
        return Err("query must not be empty".to_owned());
    }
    if request.top_k == 0 {
        return Err("top_k must be greater than zero".to_owned());
    }

    let documents = request.documents.clone().unwrap_or_else(default_documents);
    if documents.is_empty() {
        return Err("documents must not be empty".to_owned());
    }

    let texts = documents
        .iter()
        .map(|doc| doc.text.as_str())
        .chain(std::iter::once(request.query.as_str()))
        .collect::<Vec<_>>();
    let (mut embeddings, embedding_source) = embed_texts(&request.embedding_model, &texts);
    let query_embedding = embeddings
        .pop()
        .ok_or_else(|| "embedding pipeline returned no query embedding".to_owned())?;

    let _ = bindings::tachyon::mesh::vector::create_index(
        &bindings::tachyon::mesh::vector::IndexSpec {
            name: request.index.clone(),
            dim: query_embedding.len() as u32,
            m: 16,
            ef_construction: 200,
        },
    );

    let vector_docs = documents
        .iter()
        .zip(embeddings.iter())
        .map(
            |(doc, embedding)| bindings::tachyon::mesh::vector::Document {
                id: doc.id.clone(),
                embedding: embedding.clone(),
                payload: Some(doc.text.as_bytes().to_vec()),
            },
        )
        .collect::<Vec<_>>();
    bindings::tachyon::mesh::vector::upsert(&request.index, &vector_docs)
        .map_err(|error| format!("vector upsert failed: {error}"))?;

    let matches =
        bindings::tachyon::mesh::vector::search(&request.index, &query_embedding, request.top_k)
            .map_err(|error| format!("vector search failed: {error}"))?
            .into_iter()
            .map(|item| RagMatch {
                id: item.id,
                score: item.score,
                text: item
                    .payload
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    .unwrap_or_default(),
            })
            .collect::<Vec<_>>();

    let context = matches
        .iter()
        .map(|item| format!("- {}: {}", item.id, item.text))
        .collect::<Vec<_>>()
        .join("\n");
    let (answer, completion_source) =
        complete_with_context(&request.chat_model, &request.query, &context).unwrap_or_else(|| {
            (
                fallback_answer(&request.query, &matches),
                "local-fallback".to_owned(),
            )
        });

    serde_json::to_vec(&RagResponse {
        query: request.query,
        index: request.index,
        answer,
        matches,
        embedding_source,
        completion_source,
    })
    .map_err(|error| format!("failed to encode RAG response: {error}"))
}

fn embed_texts(model: &str, texts: &[&str]) -> (Vec<Vec<f32>>, String) {
    let body = serde_json::json!({
        "model": model,
        "input": texts,
    })
    .to_string()
    .into_bytes();

    if let Ok(response) = bindings::tachyon::mesh::outbound_http::send_request(
        "POST",
        "/ai/v1/embeddings",
        &[("content-type".to_owned(), "application/json".to_owned())],
        &body,
    ) {
        if response.status == 200 {
            if let Ok(decoded) = serde_json::from_slice::<OpenAiEmbeddingResponse>(&response.body) {
                let embeddings = decoded
                    .data
                    .into_iter()
                    .map(|item| normalize_embedding(item.embedding))
                    .collect::<Vec<_>>();
                let dim = embeddings.first().map(Vec::len).unwrap_or_default();
                if embeddings.len() == texts.len()
                    && dim > 0
                    && embeddings.iter().all(|embedding| embedding.len() == dim)
                {
                    return (embeddings, format!("openai-compatible:{model}"));
                }
            }
        }
    }

    (
        texts
            .iter()
            .map(|text| deterministic_embedding(text))
            .collect(),
        "local-deterministic-fallback".to_owned(),
    )
}

fn complete_with_context(model: &str, query: &str, context: &str) -> Option<(String, String)> {
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": "Answer only from the supplied Tachyon Mesh context."
            },
            {
                "role": "user",
                "content": format!("Context:\n{context}\n\nQuestion: {query}")
            }
        ],
        "max_tokens": 160
    })
    .to_string()
    .into_bytes();

    let response = bindings::tachyon::mesh::outbound_http::send_request(
        "POST",
        "/ai/v1/chat/completions",
        &[("content-type".to_owned(), "application/json".to_owned())],
        &body,
    )
    .ok()?;
    if response.status != 200 {
        return None;
    }
    let decoded = serde_json::from_slice::<OpenAiChatResponse>(&response.body).ok()?;
    decoded
        .choices
        .into_iter()
        .next()
        .map(|choice| (choice.message.content, format!("openai-compatible:{model}")))
}

fn fallback_answer(query: &str, matches: &[RagMatch]) -> String {
    let Some(best) = matches.first() else {
        return format!("No context matched `{query}`.");
    };
    format!(
        "Best context for `{query}` is `{}` with score {:.3}: {}",
        best.id, best.score, best.text
    )
}

fn deterministic_embedding(text: &str) -> Vec<f32> {
    let mut values = vec![0.0_f32; FALLBACK_EMBEDDING_DIM as usize];
    for token in text.split(|ch: char| !ch.is_alphanumeric()) {
        if token.is_empty() {
            continue;
        }
        let mut hash = 2_166_136_261_u32;
        for byte in token.bytes() {
            hash ^= u32::from(byte.to_ascii_lowercase());
            hash = hash.wrapping_mul(16_777_619);
        }
        let idx = (hash as usize) % values.len();
        values[idx] += 1.0;
    }
    normalize_embedding(values)
}

fn normalize_embedding(mut embedding: Vec<f32>) -> Vec<f32> {
    let norm = embedding
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if norm > 0.0 {
        for value in &mut embedding {
            *value /= norm;
        }
    }
    embedding
}

fn default_documents() -> Vec<InputDocument> {
    vec![
        InputDocument {
            id: "dispatch-locality".to_owned(),
            text: "Tachyon Mesh can dispatch calls between sealed FaaS routes in-process when the target route is local and not saturated.".to_owned(),
        },
        InputDocument {
            id: "vector-rag".to_owned(),
            text: "The vector WIT interface creates indexes, upserts embeddings with payloads, and returns nearest matches for retrieval augmented generation.".to_owned(),
        },
        InputDocument {
            id: "mcp-access".to_owned(),
            text: "The tachyon_vector_search MCP tool is read-only and forwards an agent query to the RAG route with an index name and top_k limit.".to_owned(),
        },
    ]
}

fn json_error(error: &str) -> Vec<u8> {
    serde_json::json!({ "error": error })
        .to_string()
        .into_bytes()
}

fn response(status: u16, body: Vec<u8>) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body,
        trailers: vec![],
    }
}

fn default_index() -> String {
    DEFAULT_INDEX.to_owned()
}

fn default_embedding_model() -> String {
    DEFAULT_EMBEDDING_MODEL.to_owned()
}

fn default_chat_model() -> String {
    DEFAULT_CHAT_MODEL.to_owned()
}

fn default_top_k() -> u32 {
    3
}
