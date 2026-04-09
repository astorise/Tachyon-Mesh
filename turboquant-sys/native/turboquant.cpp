#include "turboquant.hpp"

#include <math.h>
#include <string.h>

namespace {

const float kLevels2Bit[4] = {-1.0f, -0.33333334f, 0.33333334f, 1.0f};
const float kLevels3Bit[8] = {
    -1.0f,
    -0.71428573f,
    -0.42857143f,
    -0.14285715f,
    0.14285715f,
    0.42857143f,
    0.71428573f,
    1.0f,
};

inline const float* levels_for_bits(uint8_t bits, size_t* count) {
    if (bits == 2) {
        *count = 4;
        return kLevels2Bit;
    }
    if (bits == 3) {
        *count = 8;
        return kLevels3Bit;
    }
    *count = 0;
    return nullptr;
}

inline uint8_t nearest_code(float value, const float* levels, size_t count) {
    uint8_t best_index = 0;
    float best_distance = fabsf(value - levels[0]);
    for (size_t index = 1; index < count; ++index) {
        const float distance = fabsf(value - levels[index]);
        if (distance < best_distance) {
            best_distance = distance;
            best_index = static_cast<uint8_t>(index);
        }
    }
    return best_index;
}

inline uint8_t unpack_code(const uint8_t* packed, size_t value_index, uint8_t bits) {
    const size_t bit_index = value_index * static_cast<size_t>(bits);
    const size_t byte_index = bit_index / 8;
    const size_t intra_byte = bit_index % 8;
    const uint16_t window = static_cast<uint16_t>(packed[byte_index]) |
                            static_cast<uint16_t>(packed[byte_index + 1]) << 8;
    return static_cast<uint8_t>((window >> intra_byte) & ((1u << bits) - 1u));
}

}  // namespace

size_t tq_packed_len(size_t value_count, uint8_t bits) {
    if (bits != 2 && bits != 3) {
        return 0;
    }
    const size_t total_bits = value_count * static_cast<size_t>(bits);
    return (total_bits + 7u) / 8u;
}

int tq_compress_values_f32(
    const float* input,
    size_t value_count,
    uint8_t bits,
    uint8_t* output,
    size_t output_len
) {
    if (input == nullptr || output == nullptr) {
        return 1;
    }
    size_t level_count = 0;
    const float* levels = levels_for_bits(bits, &level_count);
    if (levels == nullptr) {
        return 2;
    }
    const size_t expected_len = tq_packed_len(value_count, bits);
    if (expected_len == 0 || output_len < expected_len) {
        return 3;
    }

    memset(output, 0, output_len);
    size_t bit_index = 0;
    for (size_t index = 0; index < value_count; ++index) {
        const uint8_t code = nearest_code(input[index], levels, level_count);
        for (uint8_t bit = 0; bit < bits; ++bit) {
            const size_t target_bit = bit_index + bit;
            const size_t target_byte = target_bit / 8;
            const size_t intra_byte = target_bit % 8;
            output[target_byte] |= static_cast<uint8_t>(((code >> bit) & 1u) << intra_byte);
        }
        bit_index += bits;
    }
    return 0;
}

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
) {
    if (packed == nullptr || attention == nullptr || output == nullptr) {
        return 1;
    }
    size_t level_count = 0;
    const float* levels = levels_for_bits(bits, &level_count);
    if (levels == nullptr) {
        return 2;
    }
    if (attention_len < value_count || output_len < value_count) {
        return 3;
    }
    const size_t expected_len = tq_packed_len(value_count, bits);
    if (expected_len == 0 || packed_len != expected_len) {
        return 4;
    }

    for (size_t index = 0; index < value_count; ++index) {
        if (attention[index] < threshold) {
            output[index] = 0.0f;
            continue;
        }
        const uint8_t code = unpack_code(packed, index, bits);
        if (code >= level_count) {
            return 5;
        }
        output[index] = levels[code];
    }
    return 0;
}
