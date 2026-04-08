## ADDED Requirements

### Requirement: TurboQuant GPU work starts with a failing Rust test harness
The TurboQuant implementation SHALL define its Rust tests, expected tensor outputs, and FFI signatures before introducing the CUDA kernel implementation.

#### Scenario: The initial test suite runs before the kernel exists
- **WHEN** the TurboQuant Rust tests are compiled against the declared FFI boundary
- **AND** the CUDA kernel is not yet implemented
- **THEN** the tests fail in a controlled and diagnosable way that captures the expected quantization contract

### Requirement: The FFI boundary for TurboQuant is typed and explicit
The host AI module SHALL expose a C-compatible launcher for the TurboQuant kernel with typed pointers for the input tensor, compressed outputs, tensor metadata, and CUDA stream.

#### Scenario: Rust invokes the TurboQuant launcher
- **WHEN** the Candle custom operator prepares device buffers for the input and output tensors
- **THEN** it calls the declared `extern "C"` launcher with the GPU pointers, tensor dimensions, and stream handle

### Requirement: TurboQuant integrates with Candle as a GPU custom operator
The TurboQuant compressor SHALL allocate its outputs on the GPU, invoke the CUDA kernel through the typed FFI boundary, and return the resulting storage objects back to Candle's execution graph.

#### Scenario: Candle executes the TurboQuant custom operator
- **WHEN** a model graph invokes the `TurboQuantCompressor`
- **THEN** the operator allocates GPU output tensors
- **AND** launches the CUDA implementation through the FFI boundary
- **AND** returns the compressed GPU storage to the calling Candle graph
