# Proposal: AI Orchestration, Candle & Multi-GPU Configuration

## Context
Tachyon Mesh executes AI models at the Edge using the `wasi-nn` standard backed by the Rust `candle` framework. While the physical assets (`.gguf`) and hardware capabilities are managed by other configuration domains, the actual runtime orchestration of these models lacks a declarative schema.

## Problem
Running Large Language Models (LLMs) requires extreme VRAM efficiency. Hardcoding inference parameters prevents advanced Edge strategies. We need the ability to configure Multi-GPU distribution (Tensor/Pipeline parallelism) and Model Sharing (LoRA multiplexing) dynamically via Tachyon-UI to maximize hardware utilization without OOM (Out Of Memory) crashes.

## Solution
Introduce the `config-ai.wit` schema. This enables GitOps-driven configuration for the `system-faas-model-broker`, allowing operators to define:
1. **Engine Execution**: Candle backend (CUDA/Metal), quantization execution, and context window limits.
2. **Multi-GPU Strategies**: How large models are sharded across available GPUs.
3. **Sharing Strategies**: Declarative routing of identity-aware requests to specific LoRA adapters while sharing a single base model in VRAM.