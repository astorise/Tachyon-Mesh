use std::{ffi::c_void, ptr::NonNull};

use anyhow::{anyhow, Result};

use super::{Nvfp4AcceleratorCapabilities, Nvfp4KernelKind, Nvfp4OutputDType};

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Nvfp4CudaLinearF32Params {
    pub(crate) input: *const f32,
    pub(crate) packed_weight: *const u8,
    pub(crate) weight_scale_e4m3: *const u8,
    pub(crate) tensor_scale: f32,
    pub(crate) output: *mut f32,
    pub(crate) m: i64,
    pub(crate) n: i64,
    pub(crate) k: i64,
    pub(crate) stream: *mut c_void,
}

#[cfg(tachyon_nvfp4_cuda_compiled)]
use std::os::raw::c_int;

#[cfg(tachyon_nvfp4_cuda_compiled)]
unsafe extern "C" {
    fn tachyon_nvfp4_cutlass_is_available() -> c_int;
    fn tachyon_nvfp4_cuda_dequantize_f32(
        packed: *const u8,
        block_scales_e4m3: *const u8,
        tensor_scale: f32,
        n_rows: i64,
        n_cols: i64,
        output: *mut f32,
        stream: *mut c_void,
    ) -> c_int;
    fn tachyon_nvfp4_cuda_dequantize_bf16(
        packed: *const u8,
        block_scales_e4m3: *const u8,
        tensor_scale: f32,
        n_rows: i64,
        n_cols: i64,
        output: *mut u16,
        stream: *mut c_void,
    ) -> c_int;
    fn tachyon_nvfp4_cutlass_linear_f32(params: Nvfp4CudaLinearF32Params) -> c_int;
}

pub(crate) struct Nvfp4CudaBackend;

impl Nvfp4CudaBackend {
    #[cfg(tachyon_nvfp4_cuda_compiled)]
    pub(crate) fn is_available() -> bool {
        unsafe { tachyon_nvfp4_cutlass_is_available() == 1 }
    }

    #[cfg(not(tachyon_nvfp4_cuda_compiled))]
    pub(crate) fn is_available() -> bool {
        false
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

    #[cfg(tachyon_nvfp4_cuda_compiled)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) unsafe fn launch_dequantize(
        packed: NonNull<u8>,
        block_scales_e4m3: NonNull<u8>,
        tensor_scale: f32,
        n_rows: usize,
        n_cols: usize,
        output: NonNull<c_void>,
        output_dtype: Nvfp4OutputDType,
        stream: *mut c_void,
    ) -> Result<()> {
        let n_rows = i64::try_from(n_rows)?;
        let n_cols = i64::try_from(n_cols)?;
        let status = match output_dtype {
            Nvfp4OutputDType::F32 => unsafe {
                tachyon_nvfp4_cuda_dequantize_f32(
                    packed.as_ptr(),
                    block_scales_e4m3.as_ptr(),
                    tensor_scale,
                    n_rows,
                    n_cols,
                    output.as_ptr().cast::<f32>(),
                    stream,
                )
            },
            Nvfp4OutputDType::BF16 => unsafe {
                tachyon_nvfp4_cuda_dequantize_bf16(
                    packed.as_ptr(),
                    block_scales_e4m3.as_ptr(),
                    tensor_scale,
                    n_rows,
                    n_cols,
                    output.as_ptr().cast::<u16>(),
                    stream,
                )
            },
        };
        cuda_status(status, "launch NVFP4 CUDA dequantization")
    }

    #[cfg(not(tachyon_nvfp4_cuda_compiled))]
    #[allow(clippy::too_many_arguments)]
    pub(crate) unsafe fn launch_dequantize(
        _packed: NonNull<u8>,
        _block_scales_e4m3: NonNull<u8>,
        _tensor_scale: f32,
        _n_rows: usize,
        _n_cols: usize,
        _output: NonNull<c_void>,
        _output_dtype: Nvfp4OutputDType,
        _stream: *mut c_void,
    ) -> Result<()> {
        Err(anyhow!(
            "NVFP4 CUDA backend was enabled but native CUDA/CUTLASS kernels were not compiled"
        ))
    }

    #[cfg(tachyon_nvfp4_cuda_compiled)]
    pub(crate) unsafe fn launch_linear_f32(params: Nvfp4CudaLinearF32Params) -> Result<()> {
        cuda_status(
            unsafe { tachyon_nvfp4_cutlass_linear_f32(params) },
            "launch NVFP4 CUDA/CUTLASS linear kernel",
        )
    }

    #[cfg(not(tachyon_nvfp4_cuda_compiled))]
    pub(crate) unsafe fn launch_linear_f32(_params: Nvfp4CudaLinearF32Params) -> Result<()> {
        Err(anyhow!(
            "NVFP4 CUDA backend was enabled but native CUDA/CUTLASS kernels were not compiled"
        ))
    }
}

#[cfg(tachyon_nvfp4_cuda_compiled)]
fn cuda_status(status: c_int, action: &str) -> Result<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(anyhow!("{action} failed with CUDA status {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

    #[test]
    fn linear_params_are_ffi_stable() {
        assert_eq!(
            align_of::<Nvfp4CudaLinearF32Params>(),
            align_of::<*const f32>()
        );
        assert!(size_of::<Nvfp4CudaLinearF32Params>() >= 72);
    }

    #[test]
    fn gated_cuda_capability_probe() {
        if std::env::var("TACHYON_NVFP4_CUDA_SMOKE").as_deref() != Ok("1") {
            return;
        }

        assert!(Nvfp4CudaBackend::is_available());
    }
}
