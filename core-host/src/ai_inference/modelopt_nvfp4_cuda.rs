//! NVFP4 execution, delegated to `candle-nvfp4-kernels`.
//!
//! This module used to carry its own CUTLASS kernel and the build-script
//! plumbing that compiled it (`src/ai_inference/cuda/nvfp4_cutlass.cu`, a
//! `TACHYON_CUTLASS_INCLUDE_DIR` probe, a `tachyon_nvfp4_cuda_compiled` cfg).
//! Candle now ships the kernel as a crate, beside the FP8, AWQ and GPTQ ones
//! (candle #3824), so what remains here is the adapter: our capability
//! vocabulary on one side, the upstream entry points on the other.
//!
//! The `nvfp4-cuda` feature stays CPU-buildable and therefore stays in the
//! standard feature matrix. `candle-nvfp4-kernels` is pulled in with its own
//! `cuda` feature *off*; `candle-cuda` is what turns it on. Without it
//! `is_available()` answers `false` and every entry point reports the
//! operator as unsupported, which is exactly what the missing CUTLASS headers
//! used to produce.

use anyhow::{anyhow, Result};

use super::{Nvfp4AcceleratorCapabilities, Nvfp4KernelKind};

pub(crate) struct Nvfp4CudaBackend;

impl Nvfp4CudaBackend {
    /// Whether this build and this device can execute FP4 natively.
    ///
    /// Delegated rather than re-derived: the kernel crate gates on compute
    /// capability 10.0, and duplicating that threshold here is how the two
    /// would eventually disagree about which devices are supported.
    pub(crate) fn is_available() -> bool {
        candle_nvfp4_kernels::is_available()
    }

    pub(crate) fn capabilities() -> Nvfp4AcceleratorCapabilities {
        if Self::is_available() {
            Nvfp4AcceleratorCapabilities::native_fp4([
                Nvfp4KernelKind::Dequantize,
                Nvfp4KernelKind::Matmul,
            ])
        } else {
            Nvfp4AcceleratorCapabilities::fallback_only()
        }
    }

    /// One activation vector through one NVFP4 operator.
    pub(crate) fn linear_f32_host(
        input: &[f32],
        packed_weight: &[u8],
        weight_scale_e4m3: &[u8],
        tensor_scale: f32,
        rows: usize,
        cols: usize,
    ) -> Result<Vec<f32>> {
        candle_nvfp4_kernels::linear_f32_host(
            input,
            packed_weight,
            weight_scale_e4m3,
            tensor_scale,
            rows,
            cols,
        )
        .map_err(|error| anyhow!("NVFP4 CUDA linear failed: {error}"))
    }

    /// `tokens` activations through one NVFP4 operator, in a single launch.
    ///
    /// This is the entry point that makes batching worth anything on a GPU.
    /// Before it existed the caller looped [`Self::linear_f32_host`], so a
    /// 1024-token prefill launched 1024 kernels per operator and re-read the
    /// weight each time — the batched host path upstream of this was doing
    /// real work and then handing it to a per-token kernel.
    pub(crate) fn linear_f32_host_batched(
        inputs: &[f32],
        packed_weight: &[u8],
        weight_scale_e4m3: &[u8],
        tensor_scale: f32,
        rows: usize,
        cols: usize,
        tokens: usize,
    ) -> Result<Vec<f32>> {
        candle_nvfp4_kernels::linear_f32_host_batched(
            inputs,
            packed_weight,
            weight_scale_e4m3,
            tensor_scale,
            rows,
            cols,
            tokens,
        )
        .map_err(|error| anyhow!("NVFP4 CUDA batched linear failed: {error}"))
    }
}
