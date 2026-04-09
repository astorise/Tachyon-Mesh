#include "../native/turboquant.hpp"

#include <array>
#include <fstream>
#include <vector>

namespace {

constexpr std::array<float, 16> kFixtureValues = {
    1.0f,
    -1.0f,
    -0.33333334f,
    0.33333334f,
    0.33333334f,
    -0.33333334f,
    1.0f,
    -1.0f,
    -0.33333334f,
    -0.33333334f,
    0.33333334f,
    1.0f,
    -1.0f,
    0.33333334f,
    1.0f,
    -1.0f,
};

template <typename T>
void write_binary(const char* path, const T* data, size_t count) {
    std::ofstream stream(path, std::ios::binary | std::ios::trunc);
    stream.write(reinterpret_cast<const char*>(data), static_cast<std::streamsize>(count * sizeof(T)));
}

}  // namespace

int main() {
    std::vector<uint8_t> packed(tq_packed_len(kFixtureValues.size(), 2));
    const int status = tq_compress_values_f32(
        kFixtureValues.data(),
        kFixtureValues.size(),
        2,
        packed.data(),
        packed.size()
    );
    if (status != 0) {
        return status;
    }

    write_binary("../fixtures/v_tensor_f32.bin", kFixtureValues.data(), kFixtureValues.size());
    write_binary("../fixtures/v_tensor_tq.bin", packed.data(), packed.size());
    return 0;
}
