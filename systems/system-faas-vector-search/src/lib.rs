mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};

struct Component;

#[derive(Debug, Deserialize)]
struct SearchRequest {
    query: Vec<f32>,
    embeddings: Vec<VectorRow>,
    #[serde(default = "default_top_k")]
    top_k: usize,
    #[serde(default = "default_m")]
    m: usize,
    #[serde(default = "default_ef_construction")]
    ef_construction: usize,
    #[serde(default = "default_ef_search")]
    ef_search: usize,
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
    let index = AnnIndex::build(
        request.embeddings,
        request.m,
        request.ef_construction.max(request.top_k),
    )?;
    let results = index.search(&request.query, request.top_k, request.ef_search)?;
    serde_json::to_vec(&results)
        .map_err(|error| format!("failed to encode vector results: {error}"))
}

struct AnnIndex {
    nodes: Vec<VectorNode>,
    graph: Vec<Vec<usize>>,
    entrypoint: Option<usize>,
    m: usize,
}

struct VectorNode {
    id: String,
    embedding: Vec<f32>,
}

impl AnnIndex {
    fn build(rows: Vec<VectorRow>, m: usize, ef_construction: usize) -> Result<Self, String> {
        let m = m.max(1);
        let mut index = Self {
            nodes: Vec::with_capacity(rows.len()),
            graph: Vec::with_capacity(rows.len()),
            entrypoint: None,
            m,
        };

        for row in rows {
            let embedding = normalize_embedding(&row.embedding)?;
            if let Some(first) = index.nodes.first() {
                validate_dim(first.embedding.len(), &embedding)?;
            }
            let next = index.nodes.len();
            let neighbors = index.search_indices(&embedding, m, ef_construction);
            index.nodes.push(VectorNode {
                id: row.id,
                embedding,
            });
            index.graph.push(Vec::new());
            for neighbor in neighbors {
                index.connect(next, neighbor);
            }
            index.entrypoint.get_or_insert(next);
        }

        Ok(index)
    }

    fn search(
        &self,
        query: &[f32],
        top_k: usize,
        ef_search: usize,
    ) -> Result<Vec<SearchResult>, String> {
        if top_k == 0 || self.nodes.is_empty() {
            return Ok(Vec::new());
        }
        let query = normalize_embedding(query)?;
        validate_dim(self.nodes[0].embedding.len(), &query)?;
        Ok(self
            .search_indices(&query, top_k, ef_search.max(top_k))
            .into_iter()
            .map(|idx| SearchResult {
                id: self.nodes[idx].id.clone(),
                score: dot(&self.nodes[idx].embedding, &query),
            })
            .collect())
    }

    fn connect(&mut self, left: usize, right: usize) {
        push_bounded_neighbor(&mut self.graph[left], right, self.m);
        push_bounded_neighbor(&mut self.graph[right], left, self.m);
    }

    fn search_indices(&self, query: &[f32], top_k: usize, ef: usize) -> Vec<usize> {
        let Some(entrypoint) = self.entrypoint else {
            return Vec::new();
        };

        let mut visited = HashSet::from([entrypoint]);
        let mut candidates = BinaryHeap::new();
        let mut nearest = BinaryHeap::new();
        let entry_score = dot(&self.nodes[entrypoint].embedding, query);
        candidates.push(Candidate::for_candidates(entry_score, entrypoint));
        nearest.push(Candidate::for_nearest(entry_score, entrypoint));

        while let Some(current) = candidates.pop() {
            let worst_score = nearest
                .peek()
                .map(|candidate| candidate.score)
                .unwrap_or(f32::NEG_INFINITY);
            if nearest.len() >= ef && current.score < worst_score {
                break;
            }

            for &neighbor in &self.graph[current.index] {
                if !visited.insert(neighbor) {
                    continue;
                }
                let score = dot(&self.nodes[neighbor].embedding, query);
                let should_keep = nearest.len() < ef
                    || nearest
                        .peek()
                        .map(|candidate| score > candidate.score)
                        .unwrap_or(true);
                if should_keep {
                    candidates.push(Candidate::for_candidates(score, neighbor));
                    nearest.push(Candidate::for_nearest(score, neighbor));
                    if nearest.len() > ef {
                        nearest.pop();
                    }
                }
            }
        }

        let mut indices = nearest
            .into_iter()
            .map(|candidate| candidate.index)
            .collect::<Vec<_>>();
        indices.sort_by(|left, right| {
            dot(&self.nodes[*right].embedding, query)
                .total_cmp(&dot(&self.nodes[*left].embedding, query))
        });
        indices.truncate(top_k);
        indices
    }
}

#[derive(Clone, Copy, Debug)]
struct Candidate {
    score: f32,
    index: usize,
    reverse: bool,
}

impl Candidate {
    fn for_candidates(score: f32, index: usize) -> Self {
        Self {
            score,
            index,
            reverse: false,
        }
    }

    fn for_nearest(score: f32, index: usize) -> Self {
        Self {
            score,
            index,
            reverse: true,
        }
    }
}

impl Eq for Candidate {}

impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.score.to_bits() == other.score.to_bits()
            && self.index == other.index
            && self.reverse == other.reverse
    }
}

impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        let ordering = self
            .score
            .total_cmp(&other.score)
            .then_with(|| self.index.cmp(&other.index));
        if self.reverse {
            ordering.reverse()
        } else {
            ordering
        }
    }
}

impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn push_bounded_neighbor(neighbors: &mut Vec<usize>, neighbor: usize, limit: usize) {
    if neighbors.contains(&neighbor) {
        return;
    }
    neighbors.push(neighbor);
    if neighbors.len() > limit {
        neighbors.remove(0);
    }
}

fn normalize_embedding(embedding: &[f32]) -> Result<Vec<f32>, String> {
    if embedding.is_empty() {
        return Err("embedding must not be empty".to_owned());
    }
    if embedding.iter().any(|value| !value.is_finite()) {
        return Err("embedding contains a non-finite value".to_owned());
    }
    let norm = embedding
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if norm == 0.0 {
        return Err("embedding norm must be non-zero".to_owned());
    }
    Ok(embedding.iter().map(|value| value / norm).collect())
}

fn validate_dim(expected: usize, embedding: &[f32]) -> Result<(), String> {
    if embedding.len() != expected {
        Err(format!(
            "embedding dimension mismatch: expected {expected}, got {}",
            embedding.len()
        ))
    } else {
        Ok(())
    }
}

fn dot(left: &[f32], right: &[f32]) -> f32 {
    let mut dot = 0.0;
    for (left, right) in left.iter().zip(right.iter()) {
        dot += left * right;
    }
    dot
}

fn default_top_k() -> usize {
    10
}

fn default_m() -> usize {
    16
}

fn default_ef_construction() -> usize {
    64
}

fn default_ef_search() -> usize {
    64
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
        let encoded = search(body).expect("valid search request should encode");
        let results: Vec<SearchResult> =
            serde_json::from_slice(&encoded).expect("search response should decode");

        assert_eq!(
            results,
            vec![SearchResult {
                id: "y".to_owned(),
                score: 1.0
            }]
        );
    }

    #[test]
    fn rejects_dimension_mismatch_before_searching() {
        let body = br#"{"query":[1,0],"embeddings":[{"id":"x","embedding":[1,0,0]}]}"#;
        let error = search(body).expect_err("mismatched dimensions should fail");

        assert!(error.contains("embedding dimension mismatch"));
    }

    #[test]
    fn honors_top_k_with_ann_parameters() {
        let body = br#"{"query":[1,0,0],"top_k":2,"m":2,"ef_construction":4,"ef_search":4,"embeddings":[{"id":"a","embedding":[1,0,0]},{"id":"b","embedding":[0.8,0.2,0]},{"id":"c","embedding":[0,1,0]}]}"#;
        let encoded = search(body).expect("valid search request should encode");
        let results: Vec<SearchResult> =
            serde_json::from_slice(&encoded).expect("search response should decode");

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].id, "a");
    }
}
