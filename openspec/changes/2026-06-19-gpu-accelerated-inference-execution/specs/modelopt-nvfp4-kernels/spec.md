# modelopt-nvfp4-kernels Delta

## MODIFIED Requirements

### Requirement: Native FP4 acceleration MUST be capability-gated
The runtime SHALL only select native NVFP4 dequant/matmul kernels when the accelerator backend reports compatible hardware, driver/runtime support, and kernel availability. When selected, the runtime SHALL execute a real forward pass against the compiled native kernels rather than returning an unsupported-execution error.

#### Scenario: Compatible backend selects native FP4 and executes a real forward pass
- **WHEN** the selected accelerator reports native FP4 capability
- **AND** required NVFP4 kernels are compiled and available
- **THEN** the runtime executes packed FP4 weights through the compiled CUDA/CUTLASS dequant/matmul kernels without eager BF16/F32 dequantization
- **AND** the resulting output is real model output, not an unsupported-execution error

#### Scenario: Unsupported accelerator falls back to a real GPU execution path or rejects
- **WHEN** the selected accelerator lacks native NVFP4 support
- **AND** fallback dequantization is allowed within configured memory limits
- **THEN** the runtime dequantizes via the existing fallback path and executes the resulting dense tensors through a standard GPU matmul
- **AND** if fallback exceeds configured limits, startup or inference fails with the existing typed unsupported-accelerator error

## ADDED Requirements

### Requirement: NVFP4 inference execution path MUST be selected deterministically and reported
For every inference call against a ModelOpt/NVFP4-classified model binding, the runtime SHALL deterministically select one of `native-fp4`, `fp32-fallback`, or `unsupported-execution` based on accelerator capability and configured memory limits, and SHALL report the selected path in telemetry.

#### Scenario: Native FP4 path is chosen when available
- **GIVEN** a model binding classified as ModelOpt/NVFP4
- **AND** the accelerator reports native FP4 capability with compiled kernels available
- **WHEN** an inference request is submitted for that binding
- **THEN** the runtime selects the `native-fp4` execution path
- **AND** telemetry records `executed_on: gpu-native-fp4`

#### Scenario: Fallback path is chosen when native kernels are unavailable but memory budget allows
- **GIVEN** a model binding classified as ModelOpt/NVFP4
- **AND** the accelerator lacks native FP4 capability
- **AND** the BF16/F32 fallback dequantization fits within the configured memory budget
- **WHEN** an inference request is submitted for that binding
- **THEN** the runtime selects the `fp32-fallback` execution path
- **AND** telemetry records `executed_on: gpu-fallback`

#### Scenario: Unsupported-execution error remains the last resort, not the default
- **GIVEN** a model binding classified as ModelOpt/NVFP4
- **AND** neither native FP4 kernels nor a memory-budget-compliant fallback are available
- **WHEN** an inference request is submitted for that binding
- **THEN** the runtime returns the existing typed unsupported-execution error
- **AND** the response is not `MOCK_LLM_RESPONSE`
