mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};

struct Component;

#[derive(Debug, Deserialize)]
struct SearchRequest {
    query: Vec<f32>,
    embeddings: Vec<VectorRow>,
    #[serde(default = "default_top_k")]
    top_k: usize,
}

#[derive(Debug, Deserialize)]
struct VectorRow {
    id: String,
    embedding: Vec<f32>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct SearchResult {
    id: String,
    score: f32,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        match search(&req.body) {
            Ok(body) => response(200, body),
            Err(error) => response(400, error),
        }
    }
}

fn search(input: &[u8]) -> Result<Vec<u8>, String> {
    let request: SearchRequest = serde_json::from_slice(input)
        .map_err(|error| format!("invalid vector request: {error}"))?;
    let mut results = request
        .embeddings
        .into_iter()
        .map(|row| SearchResult {
            id: row.id,
            score: cosine_similarity(&request.query, &row.embedding),
        })
        .collect::<Vec<_>>();
    results.sort_by(|left, right| right.score.total_cmp(&left.score));
    results.truncate(request.top_k);
    serde_json::to_vec(&results)
        .map_err(|error| format!("failed to encode vector results: {error}"))
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let mut dot = 0.0;
    let mut left_norm = 0.0;
    let mut right_norm = 0.0;
    for (left, right) in left.iter().zip(right.iter()) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm.sqrt() * right_norm.sqrt())
    }
}

fn default_top_k() -> usize {
    10
}

fn response(
    status: u16,
    body: impl Into<Vec<u8>>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: body.into(),
        trailers: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_best_cosine_match() {
        let body = br#"{"query":[1,0],"top_k":1,"embeddings":[{"id":"x","embedding":[0,1]},{"id":"y","embedding":[1,0]}]}"#;
        let results: Vec<SearchResult> = serde_json::from_slice(&search(body).unwrap()).unwrap();

        assert_eq!(
            results,
            vec![SearchResult {
                id: "y".to_owned(),
                score: 1.0
            }]
        );
    }
}
