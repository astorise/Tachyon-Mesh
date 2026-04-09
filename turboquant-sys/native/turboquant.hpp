#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

size_t tq_packed_len(size_t value_count, uint8_t bits);

int tq_compress_values_f32(
    const float* input,
    size_t value_count,
    uint8_t bits,
    uint8_t* output,
    size_t output_len
);

int tq_decompress_values_sparse_f32(
    const uint8_t* packed,
    size_t packed_len,
    size_t value_count,
    uint8_t bits,
    const float* attention,
    size_t attention_len,
    float threshold,
    float* output,
    size_t output_len
);

#ifdef __cplusplus
}
#endif
