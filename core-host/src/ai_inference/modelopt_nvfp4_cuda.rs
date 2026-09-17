//! NVFP4 execution, delegated to `candle-nvfp4-kernels`.
//!
//! This module used to carry its own CUTLASS kernel and the build-script
//! plumbing that compiled it (`src/ai_inference/cuda/nvfp4_cutlass.cu`, a
//! `TACHYON_CUTLASS_INCLUDE_DIR` probe, a `tachyon_nvfp4_cuda_compiled` cfg).
//! Candle now ships the kernel as a crate, beside the FP8, AWQ and GPTQ ones
//! (candle #3824), so what remains here is the adapter: our capability
//! vocabulary on one side, the upstream entry points on the other.
//!
//! This file is always compiled. `candle-nvfp4-kernels` comes in with
//! `ai-inference`, with its own `cuda` feature *off*; `candle-cuda` is what
//! turns it on. Without it `is_available()` answers `false` and every entry
//! point reports the operator as unsupported, which is exactly what the
//! missing CUTLASS headers used to produce — so a CPU build needs no `cfg` to
//! stay honest, it just gets told no.

use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use candle_core::{Device, Tensor};

use super::Nvfp4KernelAvailability;

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
        Self::upload_on(
            device()?,
            packed_weight,
            weight_scale_e4m3,
            tensor_scale,
            rows,
            cols,
        )
    }

    /// The same, onto a device the caller names — so an operator lands on the
    /// device its activations already live on, rather than on whichever one
    /// this process opened first.
    pub(crate) fn upload_on(
        device: &Device,
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
            device,
        )
        .ok()?;
        Some(Self { inner, cols })
    }

    /// The operator applied to a tensor already on the device.
    ///
    /// The entry point that costs nothing: no staging in, no staging out. A
    /// caller whose activations are tensors should never reach for
    /// [`Self::matmul`].
    pub(crate) fn forward(&self, xs: &Tensor) -> candle_core::Result<Tensor> {
        self.inner.forward(xs)
    }

    /// `tokens` host activations through the resident operator.
    ///
    /// For the scalar runtime, which holds its state in `Vec<f32>`: the weight
    /// stays resident but the activations still cross the bus twice. That is
    /// the cost of a host-side model, and it is why this exists beside
    /// [`Self::forward`] rather than instead of it.
    pub(crate) fn matmul(&self, inputs: &[f32], tokens: usize) -> Result<Vec<f32>> {
        let device = device().ok_or_else(|| anyhow!("NVFP4 CUDA device is no longer available"))?;
        let inputs = Tensor::from_slice(inputs, (tokens, self.cols), device)
            .map_err(|error| anyhow!("NVFP4 activation upload failed: {error}"))?;
        self.forward(&inputs)
            .and_then(|output| output.flatten_all()?.to_vec1::<f32>())
            .map_err(|error| anyhow!("NVFP4 CUDA resident linear failed: {error}"))
    }
}

pub(crate) struct Nvfp4CudaBackend;

impl Nvfp4CudaBackend {
    /// Whether the NVFP4 kernel can be called from this build.
    ///
    /// Asked, never re-derived. Whatever the kernel requires of a device is
    /// the kernel's to state — and it has already changed once: candle #3831
    /// dropped the compute-capability 10.0 floor after establishing that
    /// nothing in the kernel needs FP4 tensor cores. Any copy of that rule
    /// kept on this side would now be wrong.
    pub(crate) fn is_available() -> bool {
        candle_nvfp4_kernels::is_available()
    }

    pub(crate) fn availability() -> Nvfp4KernelAvailability {
        if Self::is_available() {
            Nvfp4KernelAvailability::Reachable
        } else {
            Nvfp4KernelAvailability::Absent
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

#[cfg(test)]
mod tests {
    use super::super::NVFP4_BLOCK_SIZE;
    use super::*;

    /// The smallest possible proof that the NVFP4 kernel is real.
    ///
    /// `Nvfp4KernelAvailability::detect()` answers from
    /// `candle_nvfp4_kernels::is_available()`, which reports whether the crate
    /// was compiled with its `cuda` feature and a device is present. Neither
    /// says the kernel module can be *loaded*: a build whose cubin carries no
    /// code for the runner's compute capability answers `Reachable` and then
    /// fails at the first launch with `CUDA_ERROR_NOT_FOUND`.
    ///
    /// That gap is not theoretical — it is what the Qwen fixture hit on the GPU
    /// runner, inside candle's model construction, where the error names a
    /// layer rather than a kernel. This calls the entry point directly with one
    /// operator and one token, so a failure separates "the kernel module is not
    /// usable on this device" from "candle cannot build this model".
    #[test]
    fn the_nvfp4_kernel_multiplies_on_a_real_device() {
        if candle_core::Device::new_cuda(0).is_err() {
            eprintln!("skipping NVFP4 kernel probe: no CUDA device");
            return;
        }
        if !Nvfp4CudaBackend::is_available() {
            eprintln!("skipping NVFP4 kernel probe: candle reports the kernel unavailable");
            return;
        }

        const ROWS: usize = 16;
        const COLS: usize = 32;
        // Both nibbles are E2M1 code 2, which is 1.0, and every block scale is
        // E4M3 1.0 — so the dequantized weight is all ones and each output is
        // the sum of the inputs. A kernel that ran but computed nothing would
        // answer zeros and be caught here rather than reported as a pass.
        let packed = vec![0x22u8; ROWS * COLS / 2];
        let scales = vec![0x38u8; ROWS * COLS / NVFP4_BLOCK_SIZE];
        let inputs = vec![1.0f32; COLS];

        let output = Nvfp4CudaBackend::linear_f32_host_batched(
            &inputs, &packed, &scales, 1.0, ROWS, COLS, 1,
        )
        .expect("the NVFP4 kernel must run on a device candle reports as available");

        assert_eq!(output.len(), ROWS);
        for value in output {
            assert!(
                (value - COLS as f32).abs() < 1e-3,
                "each row sums the inputs against a weight of ones: expected {COLS}, got {value}"
            );
        }
    }
}
