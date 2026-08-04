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

use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use candle_core::{Device, Tensor};

use super::{Nvfp4AcceleratorCapabilities, Nvfp4KernelKind};

/// The CUDA device every NVFP4 operator uploads to.
///
/// One per process rather than one per operator: `Device::new_cuda` opens a
/// context, and a model has hundreds of operators that all want the same one.
fn device() -> Option<&'static Device> {
    static DEVICE: OnceLock<Option<Device>> = OnceLock::new();
    DEVICE.get_or_init(|| Device::new_cuda(0).ok()).as_ref()
}

/// An NVFP4 operator whose weight lives on the device.
///
/// The host entry points below re-upload the packed weight and its scales on
/// every call, which for a `[rows, cols]` operator is megabytes per activation
/// — far more traffic than the activation itself. The weight never changes, so
/// this uploads it once at load and every later call moves only the tokens.
pub(crate) struct Nvfp4DeviceLinear {
    inner: candle_nvfp4_kernels::Nvfp4Linear,
    cols: usize,
}

impl std::fmt::Debug for Nvfp4DeviceLinear {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Nvfp4DeviceLinear")
    }
}

impl Clone for Nvfp4DeviceLinear {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            cols: self.cols,
        }
    }
}

impl Nvfp4DeviceLinear {
    /// Uploads one operator, or answers `None` when this build or this device
    /// cannot execute FP4 — in which case the caller keeps the host path it
    /// already had.
    pub(crate) fn upload(
        packed_weight: &[u8],
        weight_scale_e4m3: &[u8],
        tensor_scale: f32,
        rows: usize,
        cols: usize,
    ) -> Option<Self> {
        if !Nvfp4CudaBackend::is_available() {
            return None;
        }
        let inner = candle_nvfp4_kernels::Nvfp4Linear::new(
            packed_weight,
            weight_scale_e4m3,
            tensor_scale,
            rows,
            cols,
            device()?,
        )
        .ok()?;
        Some(Self { inner, cols })
    }

    /// `tokens` activations through the resident operator. Only the
    /// activations cross the bus.
    pub(crate) fn matmul(&self, inputs: &[f32], tokens: usize) -> Result<Vec<f32>> {
        let device = device().ok_or_else(|| anyhow!("NVFP4 CUDA device is no longer available"))?;
        let inputs = Tensor::from_slice(inputs, (tokens, self.cols), device)
            .map_err(|error| anyhow!("NVFP4 activation upload failed: {error}"))?;
        self.inner
            .forward(&inputs)
            .and_then(|output| output.flatten_all()?.to_vec1::<f32>())
            .map_err(|error| anyhow!("NVFP4 CUDA resident linear failed: {error}"))
    }
}

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

    /// `tokens` activations through one NVFP4 operator, in a single launch.
    ///
    /// The last host-staging path, and the one every native call falls back to
    /// when the operator could not be made resident. It re-sends the weight on
    /// every call; that is the price of not holding device memory, and it is
    /// why [`Nvfp4DeviceLinear`] exists above it.
    ///
    /// There is deliberately no single-activation sibling: one token is this
    /// with `tokens = 1`, and a separate entry point is an invitation to reach
    /// for the slow one by accident — which is exactly what the runtime used to
    /// do, launching a kernel per token through a whole prefill.
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
