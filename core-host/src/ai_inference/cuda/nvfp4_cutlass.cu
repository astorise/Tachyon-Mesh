#include <cuda_runtime.h>

#include <cstdint>
#include <math.h>

#include "cutlass/cutlass.h"

extern "C" {

struct TachyonNvfp4LinearF32Params {
  const float* input;
  const uint8_t* packed_weight;
  const uint8_t* weight_scale_e4m3;
  float tensor_scale;
  float* output;
  int64_t m;
  int64_t n;
  int64_t k;
  cudaStream_t stream;
};

}

namespace {

constexpr int kNvfp4BlockSize = 16;

__device__ __forceinline__ float e2m1_to_f32(uint8_t nibble) {
  switch (nibble & 0x0f) {
    case 0x0: return 0.0f;
    case 0x1: return 0.5f;
    case 0x2: return 1.0f;
    case 0x3: return 1.5f;
    case 0x4: return 2.0f;
    case 0x5: return 3.0f;
    case 0x6: return 4.0f;
    case 0x7: return 6.0f;
    case 0x8: return -0.0f;
    case 0x9: return -0.5f;
    case 0xa: return -1.0f;
    case 0xb: return -1.5f;
    case 0xc: return -2.0f;
    case 0xd: return -3.0f;
    case 0xe: return -4.0f;
    default: return -6.0f;
  }
}

__device__ __forceinline__ float e4m3_to_f32(uint8_t byte) {
  float sign = (byte & 0x80) ? -1.0f : 1.0f;
  int exp = static_cast<int>((byte >> 3) & 0x0f);
  uint8_t mant_bits = byte & 0x07;
  if (exp == 0x0f && mant_bits == 0x07) {
    return nanf("");
  }
  if (exp == 0) {
    return sign * (static_cast<float>(mant_bits) / 8.0f) * (1.0f / 64.0f);
  }
  return sign * (1.0f + static_cast<float>(mant_bits) / 8.0f) *
         exp2f(static_cast<float>(exp - 7));
}

__device__ __forceinline__ uint16_t f32_to_bf16_bits(float value) {
  uint32_t bits = __float_as_uint(value);
  uint32_t lsb = (bits >> 16) & 1u;
  uint32_t rounded = bits + 0x7fffu + lsb;
  return static_cast<uint16_t>(rounded >> 16);
}

__global__ void nvfp4_dequant_f32_kernel(
    const uint8_t* packed,
    const uint8_t* block_scales_e4m3,
    float tensor_scale,
    int64_t n_rows,
    int64_t n_cols,
    float* output) {
  int64_t idx = static_cast<int64_t>(blockIdx.x) * blockDim.x + threadIdx.x;
  int64_t total = n_rows * n_cols;
  if (idx >= total) {
    return;
  }

  int64_t row = idx / n_cols;
  int64_t col = idx % n_cols;
  int64_t packed_cols = n_cols / 2;
  int64_t scale_cols = n_cols / kNvfp4BlockSize;
  uint8_t packed_byte = packed[row * packed_cols + col / 2];
  uint8_t nibble = (col & 1) == 0 ? (packed_byte >> 4) : (packed_byte & 0x0f);
  float scale = e4m3_to_f32(block_scales_e4m3[row * scale_cols + col / kNvfp4BlockSize]);
  output[idx] = e2m1_to_f32(nibble) * scale * tensor_scale;
}

__global__ void nvfp4_dequant_bf16_kernel(
    const uint8_t* packed,
    const uint8_t* block_scales_e4m3,
    float tensor_scale,
    int64_t n_rows,
    int64_t n_cols,
    uint16_t* output) {
  int64_t idx = static_cast<int64_t>(blockIdx.x) * blockDim.x + threadIdx.x;
  int64_t total = n_rows * n_cols;
  if (idx >= total) {
    return;
  }

  int64_t row = idx / n_cols;
  int64_t col = idx % n_cols;
  int64_t packed_cols = n_cols / 2;
  int64_t scale_cols = n_cols / kNvfp4BlockSize;
  uint8_t packed_byte = packed[row * packed_cols + col / 2];
  uint8_t nibble = (col & 1) == 0 ? (packed_byte >> 4) : (packed_byte & 0x0f);
  float scale = e4m3_to_f32(block_scales_e4m3[row * scale_cols + col / kNvfp4BlockSize]);
  output[idx] = f32_to_bf16_bits(e2m1_to_f32(nibble) * scale * tensor_scale);
}

__global__ void nvfp4_linear_f32_kernel(TachyonNvfp4LinearF32Params params) {
  int64_t idx = static_cast<int64_t>(blockIdx.x) * blockDim.x + threadIdx.x;
  int64_t total = params.m * params.n;
  if (idx >= total) {
    return;
  }

  int64_t row = idx / params.n;
  int64_t out_col = idx % params.n;
  int64_t packed_cols = params.k / 2;
  int64_t scale_cols = params.k / kNvfp4BlockSize;
  float acc = 0.0f;

  for (int64_t k = 0; k < params.k; ++k) {
    uint8_t packed_byte = params.packed_weight[out_col * packed_cols + k / 2];
    uint8_t nibble = (k & 1) == 0 ? (packed_byte >> 4) : (packed_byte & 0x0f);
    float scale =
        e4m3_to_f32(params.weight_scale_e4m3[out_col * scale_cols + k / kNvfp4BlockSize]);
    float weight = e2m1_to_f32(nibble) * scale * params.tensor_scale;
    acc += params.input[row * params.k + k] * weight;
  }

  params.output[idx] = acc;
}

int validate_nvfp4_shape(int64_t n_rows, int64_t n_cols) {
  if (n_rows <= 0 || n_cols <= 0) {
    return cudaErrorInvalidValue;
  }
  if (n_cols % kNvfp4BlockSize != 0) {
    return cudaErrorInvalidValue;
  }
  return cudaSuccess;
}

int launch_size(int64_t elements) {
  constexpr int kThreads = 256;
  return static_cast<int>((elements + kThreads - 1) / kThreads);
}

}  // namespace

extern "C" int tachyon_nvfp4_cutlass_is_available() {
  int device = 0;
  cudaError_t status = cudaGetDevice(&device);
  if (status != cudaSuccess) {
    return 0;
  }
  cudaDeviceProp prop{};
  status = cudaGetDeviceProperties(&prop, device);
  if (status != cudaSuccess) {
    return 0;
  }
  int sm = prop.major * 10 + prop.minor;
  return sm >= 100 ? 1 : 0;
}

extern "C" int tachyon_nvfp4_cuda_dequantize_f32(
    const uint8_t* packed,
    const uint8_t* block_scales_e4m3,
    float tensor_scale,
    int64_t n_rows,
    int64_t n_cols,
    float* output,
    cudaStream_t stream) {
  int validation = validate_nvfp4_shape(n_rows, n_cols);
  if (validation != cudaSuccess || packed == nullptr || block_scales_e4m3 == nullptr ||
      output == nullptr) {
    return cudaErrorInvalidValue;
  }
  int64_t total = n_rows * n_cols;
  nvfp4_dequant_f32_kernel<<<launch_size(total), 256, 0, stream>>>(
      packed, block_scales_e4m3, tensor_scale, n_rows, n_cols, output);
  return cudaGetLastError();
}

extern "C" int tachyon_nvfp4_cuda_dequantize_bf16(
    const uint8_t* packed,
    const uint8_t* block_scales_e4m3,
    float tensor_scale,
    int64_t n_rows,
    int64_t n_cols,
    uint16_t* output,
    cudaStream_t stream) {
  int validation = validate_nvfp4_shape(n_rows, n_cols);
  if (validation != cudaSuccess || packed == nullptr || block_scales_e4m3 == nullptr ||
      output == nullptr) {
    return cudaErrorInvalidValue;
  }
  int64_t total = n_rows * n_cols;
  nvfp4_dequant_bf16_kernel<<<launch_size(total), 256, 0, stream>>>(
      packed, block_scales_e4m3, tensor_scale, n_rows, n_cols, output);
  return cudaGetLastError();
}

extern "C" int tachyon_nvfp4_cutlass_linear_f32(TachyonNvfp4LinearF32Params params) {
  if (validate_nvfp4_shape(params.n, params.k) != cudaSuccess || params.input == nullptr ||
      params.packed_weight == nullptr || params.weight_scale_e4m3 == nullptr ||
      params.output == nullptr) {
    return cudaErrorInvalidValue;
  }
  int64_t total = params.m * params.n;
  nvfp4_linear_f32_kernel<<<launch_size(total), 256, 0, params.stream>>>(params);
  return cudaGetLastError();
}
