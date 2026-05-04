# Design: Hardware Capabilities Data Model

## 1. The GitOps YAML Specification
This configuration allows operators to bind specific workloads to hardware accelerators.

    api_version: hardware.tachyon.io/v1alpha1
    kind: HardwareCapabilities
    metadata:
      name: edge-ai-secure-node
      environment: production
      
    spec:
      # 1. NETWORK ACCELERATION (eBPF / XDP)
      network_acceleration:
        ebpf_xdp:
          enabled: true
          mode: native # Options: native (driver), generic (skb)
          offload_l4_routing: true # Pushes L4 routing directly into the kernel
          
      # 2. COMPUTE ACCELERATION (GPU/TPU for AI Inference)
      compute_acceleration:
        ai_coprocessor:
          type: tpu
          memory_allocation_mb: 8192
          exclusive_access: false # Allows Wasm instances to share the accelerator
          
      # 3. CONFIDENTIAL COMPUTING (Secure Enclaves)
      confidential_computing:
        tee:
          enabled: true
          provider: amd_sev # Options: intel_sgx, amd_sev, aws_nitro, arm_cca
          attestation_endpoint: "https://attest.tachyon.local/verify"
          strict_enforcement: true # If true, node refuses to boot if TEE is unavailable

## 2. The WIT Contract (`wit/config-hardware.wit`)
The strict Wasm interface used by `system-faas-config-api` to safely negotiate hardware requirements with the host OS.

    interface config-hardware {
        enum xdp-mode { native, generic }
        enum accelerator-type { gpu, tpu, npu, fpga }
        enum tee-provider { intel-sgx, amd-sev, aws-nitro, arm-cca }

        record ebpf-config {
            enabled: bool,
            mode: xdp-mode,
            offload-l4: bool,
        }

        record coprocessor-config {
            device-type: accelerator-type,
            memory-mb: u32,
            exclusive: bool,
        }

        record tee-config {
            enabled: bool,
            provider: tee-provider,
            attestation-endpoint: option<string>,
            strict-enforcement: bool,
        }

        record hardware-configuration {
            network: option<ebpf-config>,
            compute: option<coprocessor-config>,
            confidential: option<tee-config>,
        }

        /// Validation (Ensures enums match requested hardware specs)
        validate-hardware-config: func(config: hardware-configuration) -> result<_, string>;

        /// CRUD Operations for Tachyon-UI / MCP
        get-hardware-config: func() -> result<hardware-configuration, string>;
        update-hardware: func(config: hardware-configuration) -> result<_, string>;
    }