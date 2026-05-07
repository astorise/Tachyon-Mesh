# Proposal: Tachyon-UI AI Orchestration Dashboard

## Context
With the App Shell and Routing views established, the next critical interface is the AI Orchestration Dashboard. This screen allows operators to deploy Large Language Models (LLMs) to the Edge, configure hardware constraints (VRAM, Tensor Parallelism), and define dynamic LoRA hot-swapping rules.

## Problem
Configuring AI models involves complex nested data (Engine parameters, Hardware strategies, LoRA arrays). Without a Virtual DOM, we must carefully structure the HTML template and Vanilla JS controller to ensure the user can intuitively build these rules and that the resulting payload strictly matches the `AiOrchestration` GitOps schema.

## Solution
Implement the AI Orchestration View.
1. **Visual Language**: Use Fuchsia/Purple accents to visually distinguish AI compute workloads from standard networking.
2. **Layout**: A grid of "Base Model" cards, with an expandable section for "LoRA Multiplexing Rules".
3. **Vanilla JS Binding**: A controller that animates interactions via GSAP and serializes the DOM state into the strict `config-ai.wit` JSON payload.