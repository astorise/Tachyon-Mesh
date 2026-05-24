# Proposal: OpenAI-Compatible Model Registry

## Why
Tachyon-Mesh is rapidly becoming an AI-native infrastructure. To maximize developer adoption, we must support the industry-standard "OpenAI API" interface. This allows developers to use existing SDKs, LangChain, or UI components without writing custom Tachyon-specific drivers.

## Problem
1. **Developer Friction:** Forcing developers to use non-standard Tachyon WIT contracts for inference limits adoption.
2. **Core Bloat:** Integrating API compatibility directly into the `core-host` would compromise its microsecond-latency L4/L7 core with HTTP/JSON parsing overhead.

## Proposed Solution
Decouple API compatibility from the host using two specialized system FaaS:
1. **system-faas-ai-list-model:** A system FaaS that maintains a persistent registry of available models (via RedDB K/V). It subscribes to the `system-faas-model-broker` to update its registry in real-time.
2. **system-faas-openai-adapter:** A system FaaS that implements the `/v1/models` and `/v1/chat/completions` REST endpoints. It fetches the model list from the registry and translates it into the OpenAI JSON schema.

## Impact
- **Ecosystem Compatibility:** Seamless integration with standard LLM tooling.
- **Pure Host:** The `core-host` remains lightweight, acting only as the secure transport layer for these FaaS-provided APIs.
