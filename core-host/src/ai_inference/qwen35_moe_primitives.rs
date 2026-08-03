use std::collections::BTreeMap;

use anyhow::{bail, Result};

use super::LayerType;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FullAttentionState {
    pub(crate) keys: Vec<Vec<f32>>,
    pub(crate) values: Vec<Vec<f32>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LinearAttentionState {
    pub(crate) conv: Vec<Vec<f32>>,
    pub(crate) recurrent: Vec<Vec<f32>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LayerDecodeState {
    Full(FullAttentionState),
    Linear(LinearAttentionState),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HybridDecodeState {
    pub(crate) layers: Vec<LayerDecodeState>,
}

impl HybridDecodeState {
    pub(crate) fn new(layer_types: &[LayerType]) -> Self {
        Self {
            layers: layer_types
                .iter()
                .map(|layer| match layer {
                    LayerType::FullAttention => {
                        LayerDecodeState::Full(FullAttentionState::default())
                    }
                    LayerType::LinearAttention => {
                        LayerDecodeState::Linear(LinearAttentionState::default())
                    }
                })
                .collect(),
        }
    }

    pub(crate) fn resident_bytes(&self) -> u64 {
        self.layers
            .iter()
            .map(|layer| match layer {
                LayerDecodeState::Full(state) => state
                    .keys
                    .iter()
                    .chain(&state.values)
                    .map(|values| (values.len() as u64).saturating_mul(4))
                    .sum::<u64>(),
                LayerDecodeState::Linear(state) => state
                    .conv
                    .iter()
                    .chain(&state.recurrent)
                    .map(|values| (values.len() as u64).saturating_mul(4))
                    .sum::<u64>(),
            })
            .sum::<u64>()
    }
}

pub(crate) fn rms_norm_qwen(input: &[f32], weight: &[f32], eps: f32) -> Result<Vec<f32>> {
    if input.len() != weight.len() || input.is_empty() {
        bail!("RMSNorm input and weight dimensions must match and be non-empty");
    }
    let mean_square = input.iter().map(|value| value * value).sum::<f32>() / input.len() as f32;
    let inv = 1.0 / (mean_square + eps).sqrt();
    Ok(input
        .iter()
        .zip(weight)
        .map(|(value, weight)| value * inv * (1.0 + weight))
        .collect())
}

pub(crate) fn silu(value: f32) -> f32 {
    value / (1.0 + (-value).exp())
}

pub(crate) fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value).exp())
}

pub(crate) fn route_top_k(logits: &[f32], top_k: usize) -> Result<Vec<(usize, f32)>> {
    if top_k == 0 || top_k > logits.len() {
        bail!("top-k must be in 1..={}", logits.len());
    }
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut probabilities = logits
        .iter()
        .enumerate()
        .map(|(index, value)| (index, (*value - max).exp()))
        .collect::<Vec<_>>();
    let total = probabilities.iter().map(|(_, value)| value).sum::<f32>();
    for (_, value) in &mut probabilities {
        *value /= total;
    }
    probabilities.sort_by(|(left_index, left), (right_index, right)| {
        right
            .total_cmp(left)
            .then_with(|| left_index.cmp(right_index))
    });
    probabilities.truncate(top_k);
    let selected_total = probabilities.iter().map(|(_, value)| value).sum::<f32>();
    for (_, value) in &mut probabilities {
        *value /= selected_total;
    }
    Ok(probabilities)
}

/// Run the sparse mixture over `tokens` activations at once.
///
/// Routing is per token — that is what a mixture is — but the *operators* are
/// not: several tokens in a chunk pick the same expert, and running them
/// separately decodes that expert's weights once per token. So the routes are
/// resolved first, the tokens that chose an expert are handed to it together,
/// and only then is the mixture accumulated.
///
/// The accumulation stays per token and in route order, which is what keeps
/// this bit-identical to running the same tokens one at a time. Chunk size is
/// a performance knob; a prefill that changed the answer when the chunk grew
/// would be a bug wearing an optimization's clothes.
///
/// `hidden` is `[tokens, hidden_size]` row-major, `router_logits` is
/// `[tokens, num_experts]`, and `experts` receives `(expert, [n, hidden_size],
/// n)` for the `n` tokens routed to it.
pub(crate) fn sparse_moe_forward_batch(
    hidden: &[f32],
    router_logits: &[f32],
    tokens: usize,
    top_k: usize,
    mut experts: impl FnMut(usize, &[f32], usize) -> Result<Vec<f32>>,
    shared_expert: impl FnOnce(&[f32], usize) -> Result<Vec<f32>>,
    shared_gate_logits: &[f32],
) -> Result<Vec<f32>> {
    if tokens == 0 {
        bail!("sparse mixture requires at least one token");
    }
    if hidden.is_empty() || !hidden.len().is_multiple_of(tokens) {
        bail!(
            "sparse mixture hidden state has {} values, expected a multiple of {tokens}",
            hidden.len()
        );
    }
    if !router_logits.len().is_multiple_of(tokens) {
        bail!(
            "sparse mixture router has {} logits, expected a multiple of {tokens}",
            router_logits.len()
        );
    }
    if shared_gate_logits.len() != tokens {
        bail!(
            "sparse mixture shared gate has {} logits, expected {tokens}",
            shared_gate_logits.len()
        );
    }
    let hidden_size = hidden.len() / tokens;
    let router_width = router_logits.len() / tokens;

    let routes = (0..tokens)
        .map(|token| {
            route_top_k(
                &router_logits[token * router_width..(token + 1) * router_width],
                top_k,
            )
        })
        .collect::<Result<Vec<_>>>()?;

    // Ascending by construction, which is what lets the accumulation below find
    // a token's row in an expert's output by binary search.
    let mut members: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (token, routes) in routes.iter().enumerate() {
        for (expert, _) in routes {
            members.entry(*expert).or_default().push(token);
        }
    }

    let mut computed: BTreeMap<usize, Vec<f32>> = BTreeMap::new();
    for (expert, members) in &members {
        let mut inputs = Vec::with_capacity(members.len() * hidden_size);
        for &token in members {
            inputs.extend_from_slice(&hidden[token * hidden_size..(token + 1) * hidden_size]);
        }
        let values = experts(*expert, &inputs, members.len())?;
        if values.len() != members.len() * hidden_size {
            bail!("expert {expert} returned an invalid hidden dimension");
        }
        computed.insert(*expert, values);
    }

    let shared = shared_expert(hidden, tokens)?;
    if shared.len() != hidden.len() {
        bail!("shared expert returned an invalid hidden dimension");
    }

    let mut output = vec![0.0f32; hidden.len()];
    for token in 0..tokens {
        let row = &mut output[token * hidden_size..(token + 1) * hidden_size];
        for (expert, weight) in &routes[token] {
            let (Some(members), Some(values)) = (members.get(expert), computed.get(expert)) else {
                bail!("expert {expert} was routed to but never executed");
            };
            let Ok(index) = members.binary_search(&token) else {
                bail!("expert {expert} was not given token {token}");
            };
            let values = &values[index * hidden_size..(index + 1) * hidden_size];
            for (output, value) in row.iter_mut().zip(values) {
                *output += value * weight;
            }
        }
        let gate = sigmoid(shared_gate_logits[token]);
        let values = &shared[token * hidden_size..(token + 1) * hidden_size];
        for (output, value) in row.iter_mut().zip(values) {
            *output += value * gate;
        }
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn full_attention_step(
    query_and_gate: &[f32],
    key: &[f32],
    value: &[f32],
    q_norm_weight: &[f32],
    k_norm_weight: &[f32],
    num_heads: usize,
    num_kv_heads: usize,
    head_dim: usize,
    rotary_dim: usize,
    rope_theta: f32,
    position: usize,
    state: &mut FullAttentionState,
) -> Result<Vec<f32>> {
    let query_width = num_heads * head_dim;
    let kv_width = num_kv_heads * head_dim;
    if query_and_gate.len() != query_width * 2
        || key.len() != kv_width
        || value.len() != kv_width
        || q_norm_weight.len() != head_dim
        || k_norm_weight.len() != head_dim
        || !num_heads.is_multiple_of(num_kv_heads)
        || rotary_dim > head_dim
        || !rotary_dim.is_multiple_of(2)
    {
        bail!("invalid full-attention dimensions");
    }
    let (query, gate) = query_and_gate.split_at(query_width);
    let mut normalized_q = Vec::with_capacity(query_width);
    for head in query.chunks_exact(head_dim) {
        normalized_q.extend(rms_norm_qwen(head, q_norm_weight, 1e-6)?);
    }
    let mut normalized_k = Vec::with_capacity(kv_width);
    for head in key.chunks_exact(head_dim) {
        normalized_k.extend(rms_norm_qwen(head, k_norm_weight, 1e-6)?);
    }
    apply_partial_rotary(
        &mut normalized_q,
        num_heads,
        head_dim,
        rotary_dim,
        rope_theta,
        position,
    );
    apply_partial_rotary(
        &mut normalized_k,
        num_kv_heads,
        head_dim,
        rotary_dim,
        rope_theta,
        position,
    );
    state.keys.push(normalized_k);
    state.values.push(value.to_vec());

    let groups = num_heads / num_kv_heads;
    let scale = 1.0 / (head_dim as f32).sqrt();
    let mut output = vec![0.0f32; query_width];
    for head in 0..num_heads {
        let kv_head = head / groups;
        let q = &normalized_q[head * head_dim..(head + 1) * head_dim];
        let mut scores = state
            .keys
            .iter()
            .map(|token| {
                let k = &token[kv_head * head_dim..(kv_head + 1) * head_dim];
                q.iter().zip(k).map(|(q, k)| q * k).sum::<f32>() * scale
            })
            .collect::<Vec<_>>();
        softmax_in_place(&mut scores);
        for (token_index, score) in scores.into_iter().enumerate() {
            let v = &state.values[token_index][kv_head * head_dim..(kv_head + 1) * head_dim];
            for dim in 0..head_dim {
                output[head * head_dim + dim] += score * v[dim];
            }
        }
    }
    for (output, gate) in output.iter_mut().zip(gate) {
        *output *= sigmoid(*gate);
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn gated_delta_step(
    projected_qkv: &[f32],
    z: &[f32],
    b: &[f32],
    a: &[f32],
    conv_weights: &[f32],
    a_log: &[f32],
    dt_bias: &[f32],
    norm_weight: &[f32],
    num_key_heads: usize,
    num_value_heads: usize,
    key_head_dim: usize,
    value_head_dim: usize,
    conv_kernel: usize,
    state: &mut LinearAttentionState,
) -> Result<Vec<f32>> {
    let key_dim = num_key_heads * key_head_dim;
    let value_dim = num_value_heads * value_head_dim;
    let conv_dim = key_dim * 2 + value_dim;
    if projected_qkv.len() != conv_dim
        || z.len() != value_dim
        || b.len() != num_value_heads
        || a.len() != num_value_heads
        || conv_weights.len() != conv_dim * conv_kernel
        || a_log.len() != num_value_heads
        || dt_bias.len() != num_value_heads
        || norm_weight.len() != value_head_dim
        || !num_value_heads.is_multiple_of(num_key_heads)
        || conv_kernel == 0
    {
        bail!("invalid gated-delta dimensions");
    }
    if state.conv.is_empty() {
        state.conv = vec![vec![0.0; conv_kernel.saturating_sub(1)]; conv_dim];
    }
    let mut mixed = vec![0.0f32; conv_dim];
    for channel in 0..conv_dim {
        let history = &mut state.conv[channel];
        let mut window = history.clone();
        window.push(projected_qkv[channel]);
        mixed[channel] = silu(
            window
                .iter()
                .zip(&conv_weights[channel * conv_kernel..(channel + 1) * conv_kernel])
                .map(|(input, weight)| input * weight)
                .sum(),
        );
        if conv_kernel > 1 {
            history.copy_from_slice(&window[1..]);
        }
    }
    let (query, tail) = mixed.split_at(key_dim);
    let (key, value) = tail.split_at(key_dim);
    if state.recurrent.is_empty() {
        state.recurrent = vec![vec![0.0; key_head_dim * value_head_dim]; num_value_heads];
    }
    let repeat = num_value_heads / num_key_heads;
    let mut output = vec![0.0f32; value_dim];
    for value_head in 0..num_value_heads {
        let key_head = value_head / repeat;
        let q = l2_normalize(&query[key_head * key_head_dim..(key_head + 1) * key_head_dim]);
        let k = l2_normalize(&key[key_head * key_head_dim..(key_head + 1) * key_head_dim]);
        let v = &value[value_head * value_head_dim..(value_head + 1) * value_head_dim];
        let beta = sigmoid(b[value_head]);
        let decay =
            (-a_log[value_head].exp() * softplus(a[value_head] + dt_bias[value_head])).exp();
        let recurrent = &mut state.recurrent[value_head];
        for element in recurrent.iter_mut() {
            *element *= decay;
        }
        let mut memory = vec![0.0f32; value_head_dim];
        for key_dim_index in 0..key_head_dim {
            for value_dim_index in 0..value_head_dim {
                memory[value_dim_index] +=
                    recurrent[key_dim_index * value_head_dim + value_dim_index] * k[key_dim_index];
            }
        }
        let delta = v
            .iter()
            .zip(memory)
            .map(|(value, memory)| (value - memory) * beta)
            .collect::<Vec<_>>();
        for key_dim_index in 0..key_head_dim {
            for value_dim_index in 0..value_head_dim {
                recurrent[key_dim_index * value_head_dim + value_dim_index] +=
                    k[key_dim_index] * delta[value_dim_index];
            }
        }
        let head_output =
            &mut output[value_head * value_head_dim..(value_head + 1) * value_head_dim];
        for key_dim_index in 0..key_head_dim {
            for value_dim_index in 0..value_head_dim {
                head_output[value_dim_index] +=
                    recurrent[key_dim_index * value_head_dim + value_dim_index] * q[key_dim_index]
                        / (key_head_dim as f32).sqrt();
            }
        }
        let normalized = rms_norm_plain(head_output, norm_weight, 1e-6)?;
        for (index, normalized) in normalized.into_iter().enumerate() {
            head_output[index] = normalized * silu(z[value_head * value_head_dim + index]);
        }
    }
    Ok(output)
}

fn rms_norm_plain(input: &[f32], weight: &[f32], eps: f32) -> Result<Vec<f32>> {
    if input.len() != weight.len() || input.is_empty() {
        bail!("gated RMSNorm dimensions must match");
    }
    let mean_square = input.iter().map(|value| value * value).sum::<f32>() / input.len() as f32;
    let inv = 1.0 / (mean_square + eps).sqrt();
    Ok(input
        .iter()
        .zip(weight)
        .map(|(value, weight)| value * inv * weight)
        .collect())
}

fn apply_partial_rotary(
    values: &mut [f32],
    heads: usize,
    head_dim: usize,
    rotary_dim: usize,
    theta: f32,
    position: usize,
) {
    for head in 0..heads {
        let offset = head * head_dim;
        for pair in 0..rotary_dim / 2 {
            let frequency = theta.powf(-((2 * pair) as f32) / rotary_dim as f32);
            let angle = position as f32 * frequency;
            let left = values[offset + pair];
            let right = values[offset + rotary_dim / 2 + pair];
            values[offset + pair] = left * angle.cos() - right * angle.sin();
            values[offset + rotary_dim / 2 + pair] = right * angle.cos() + left * angle.sin();
        }
    }
}

fn softmax_in_place(values: &mut [f32]) {
    let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut total = 0.0;
    for value in values.iter_mut() {
        *value = (*value - max).exp();
        total += *value;
    }
    for value in values {
        *value /= total;
    }
}

fn l2_normalize(values: &[f32]) -> Vec<f32> {
    let inv = 1.0 / (values.iter().map(|value| value * value).sum::<f32>() + 1e-6).sqrt();
    values.iter().map(|value| value * inv).collect()
}

fn softplus(value: f32) -> f32 {
    if value > 20.0 {
        value
    } else {
        (1.0 + value.exp()).ln()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_ties_are_deterministic_and_renormalized() {
        let routes = route_top_k(&[1.0, 1.0, 0.0], 2).expect("route");
        assert_eq!(routes[0].0, 0);
        assert_eq!(routes[1].0, 1);
        assert!((routes.iter().map(|(_, weight)| weight).sum::<f32>() - 1.0).abs() < 1e-6);
    }

    /// A stand-in expert: deterministic, distinguishable per expert, and
    /// elementwise, so batching it can only differ from the per-token form if
    /// the mixture itself does.
    fn stub_expert(expert: usize, inputs: &[f32], _tokens: usize) -> Result<Vec<f32>> {
        Ok(inputs
            .iter()
            .map(|value| value * (expert + 1) as f32 + expert as f32)
            .collect())
    }

    #[test]
    fn sparse_moe_executes_only_selected_experts_and_shared_expert() {
        let mut visited = Vec::new();
        let output = sparse_moe_forward_batch(
            &[1.0, 2.0],
            &[4.0, 3.0, -2.0],
            1,
            2,
            |expert, inputs, tokens| {
                visited.push(expert);
                stub_expert(expert, inputs, tokens)
            },
            |hidden, _| Ok(hidden.to_vec()),
            &[0.0],
        )
        .expect("MoE");
        assert_eq!(visited, vec![0, 1]);
        assert_eq!(output.len(), 2);
    }

    #[test]
    fn sparse_moe_rejects_invalid_expert_components() {
        let error = sparse_moe_forward_batch(
            &[1.0, 2.0],
            &[1.0, 0.0],
            1,
            1,
            |_expert, _inputs, _tokens| Ok(vec![1.0]),
            |hidden, _| Ok(hidden.to_vec()),
            &[0.0],
        )
        .expect_err("invalid expert output must fail");
        assert!(error.to_string().contains("invalid hidden dimension"));
    }

    /// The safety argument for chunked prefill. Grouping tokens by expert
    /// changes which activations an operator sees together; it must not change
    /// a single bit of what comes out, or a longer chunk would quietly be a
    /// different model.
    #[test]
    fn a_batched_mixture_is_bit_identical_to_one_token_at_a_time() {
        const TOKENS: usize = 7;
        const HIDDEN: usize = 5;
        const EXPERTS: usize = 6;

        // Deterministic and deliberately uneven, so the routes overlap on some
        // experts and leave others unused.
        let hidden = (0..TOKENS * HIDDEN)
            .map(|index| ((index * 37 % 23) as f32 - 11.0) / 7.0)
            .collect::<Vec<f32>>();
        let router = (0..TOKENS * EXPERTS)
            .map(|index| ((index * 53 % 19) as f32 - 9.0) / 3.0)
            .collect::<Vec<f32>>();
        let gates = (0..TOKENS)
            .map(|token| (token as f32 - 3.0) / 2.0)
            .collect::<Vec<f32>>();

        let batched = sparse_moe_forward_batch(
            &hidden,
            &router,
            TOKENS,
            2,
            stub_expert,
            |inputs, _| Ok(inputs.iter().map(|value| value * 0.5).collect()),
            &gates,
        )
        .expect("batched MoE");

        let mut one_at_a_time = Vec::with_capacity(hidden.len());
        for token in 0..TOKENS {
            one_at_a_time.extend(
                sparse_moe_forward_batch(
                    &hidden[token * HIDDEN..(token + 1) * HIDDEN],
                    &router[token * EXPERTS..(token + 1) * EXPERTS],
                    1,
                    2,
                    stub_expert,
                    |inputs, _| Ok(inputs.iter().map(|value| value * 0.5).collect()),
                    &gates[token..token + 1],
                )
                .expect("single-token MoE"),
            );
        }

        assert_eq!(batched, one_at_a_time);
    }

    #[test]
    fn a_batched_mixture_decodes_each_expert_once_for_the_whole_chunk() {
        const TOKENS: usize = 8;
        const HIDDEN: usize = 3;

        // Every token routes to the same single expert, which is the case that
        // makes the saving visible: one call, not eight.
        let hidden = vec![0.25f32; TOKENS * HIDDEN];
        let router = [1.0f32, 0.0].repeat(TOKENS);
        let mut calls = Vec::new();
        sparse_moe_forward_batch(
            &hidden,
            &router,
            TOKENS,
            1,
            |expert, inputs, tokens| {
                calls.push((expert, tokens));
                stub_expert(expert, inputs, tokens)
            },
            |inputs, _| Ok(inputs.to_vec()),
            &[0.0; TOKENS],
        )
        .expect("MoE");
        assert_eq!(calls, vec![(0, TOKENS)]);
    }

    #[test]
    fn hybrid_state_separates_full_and_linear_attention() {
        let state = HybridDecodeState::new(&[LayerType::LinearAttention, LayerType::FullAttention]);
        assert!(matches!(state.layers[0], LayerDecodeState::Linear(_)));
        assert!(matches!(state.layers[1], LayerDecodeState::Full(_)));
        assert_eq!(state.resident_bytes(), 0);
    }

    #[test]
    fn full_attention_reuses_cached_keys_and_values() {
        let mut state = FullAttentionState::default();
        let first = full_attention_step(
            &[1.0, 0.0, 8.0, 8.0],
            &[1.0, 0.0],
            &[2.0, 3.0],
            &[0.0, 0.0],
            &[0.0, 0.0],
            1,
            1,
            2,
            2,
            10_000.0,
            0,
            &mut state,
        )
        .expect("first");
        let second = full_attention_step(
            &[0.0, 1.0, 8.0, 8.0],
            &[0.0, 1.0],
            &[4.0, 5.0],
            &[0.0, 0.0],
            &[0.0, 0.0],
            1,
            1,
            2,
            2,
            10_000.0,
            1,
            &mut state,
        )
        .expect("second");
        assert_eq!(state.keys.len(), 2);
        assert_ne!(first, second);
        assert!(first[0] > 1.99 && first[0] < 2.0);
        assert!(first[1] > 2.99 && first[1] < 3.0);
    }

    #[test]
    fn gated_delta_updates_convolution_and_recurrent_state() {
        let mut state = LinearAttentionState::default();
        let output = gated_delta_step(
            &[1.0, 1.0, 2.0],
            &[1.0],
            &[0.0],
            &[0.0],
            &[1.0, 1.0, 1.0],
            &[0.0],
            &[0.0],
            &[1.0],
            1,
            1,
            1,
            1,
            1,
            &mut state,
        )
        .expect("delta");
        assert_eq!(output.len(), 1);
        assert_eq!(state.recurrent.len(), 1);
        assert_ne!(state.recurrent[0][0], 0.0);
        assert!((output[0] - 0.731_058_6).abs() < 1e-4);
    }
}
