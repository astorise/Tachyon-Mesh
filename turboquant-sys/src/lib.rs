//! Pure-Rust port of the TurboQuant 2-/3-bit quantizer.
//!
//! Previously this crate was a thin FFI shim over a C++ shared library
//! (`turboquant_native`). The C++ build was a portability and memory-safety
//! liability: cross-compilation needed a C++ toolchain, the `unsafe extern "C"`
//! boundary spanned every quantize / dequantize call, and the algorithm itself
//! was small enough that a native port pays for itself many times over.
//!
//! The public surface (`packed_len`, `compress_values`,
//! `decompress_values_sparse`, `TurboQuantError`) is byte-for-byte the same as
//! the previous FFI version, so callers under `core-host/src/ai_inference.rs`
//! need no changes.

use std::{error::Error, fmt};

/// 2-bit codebook. Reproduces the C++ `kLevels2Bit` table verbatim.
const LEVELS_2_BIT: [f32; 4] = [-1.0, -0.333_333_34, 0.333_333_34, 1.0];

/// 3-bit codebook. Reproduces the C++ `kLevels3Bit` table verbatim.
const LEVELS_3_BIT: [f32; 8] = [
    -1.0,
    -0.714_285_73,
    -0.428_571_43,
    -0.142_857_15,
    0.142_857_15,
    0.428_571_43,
    0.714_285_73,
    1.0,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurboQuantError {
    UnsupportedBitRate(u8),
    AttentionLengthMismatch { expected: usize, actual: usize },
    PackedLengthMismatch { expected: usize, actual: usize },
}

impl fmt::Display for TurboQuantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedBitRate(bits) => write!(
                f,
                "TurboQuant only supports 2-bit or 3-bit packing, got {bits}"
            ),
            Self::AttentionLengthMismatch { expected, actual } => write!(
                f,
                "TurboQuant sparse decode expects {expected} attention weights but received {actual}"
            ),
            Self::PackedLengthMismatch { expected, actual } => write!(
                f,
                "TurboQuant packed payload should be {expected} bytes but received {actual}"
            ),
        }
    }
}

impl Error for TurboQuantError {}

/// Codebook for `bits`. `None` for unsupported bit rates.
fn levels_for_bits(bits: u8) -> Option<&'static [f32]> {
    match bits {
        2 => Some(&LEVELS_2_BIT),
        3 => Some(&LEVELS_3_BIT),
        _ => None,
    }
}

fn ensure_supported_bits(bits: u8) -> Result<(), TurboQuantError> {
    if levels_for_bits(bits).is_some() {
        Ok(())
    } else {
        Err(TurboQuantError::UnsupportedBitRate(bits))
    }
}

/// Number of bytes required to pack `value_count` values at `bits` bits each.
pub fn packed_len(value_count: usize, bits: u8) -> Result<usize, TurboQuantError> {
    ensure_supported_bits(bits)?;
    Ok(packed_len_unchecked(value_count, bits))
}

fn packed_len_unchecked(value_count: usize, bits: u8) -> usize {
    let total_bits = value_count.saturating_mul(bits as usize);
    total_bits.div_ceil(8)
}

fn nearest_code(value: f32, levels: &[f32]) -> u8 {
    debug_assert!(!levels.is_empty(), "codebook must not be empty");
    let mut best_index: u8 = 0;
    let mut best_distance = (value - levels[0]).abs();
    for (i, &level) in levels.iter().enumerate().skip(1) {
        let d = (value - level).abs();
        if d < best_distance {
            best_distance = d;
            best_index = i as u8;
        }
    }
    best_index
}

/// Quantize `input` into a packed bitstream at the given bit rate.
pub fn compress_values(input: &[f32], bits: u8) -> Result<Vec<u8>, TurboQuantError> {
    let levels = levels_for_bits(bits).ok_or(TurboQuantError::UnsupportedBitRate(bits))?;
    let mut output = vec![0u8; packed_len_unchecked(input.len(), bits)];

    let mut bit_index: usize = 0;
    for &value in input {
        let code = nearest_code(value, levels);
        for bit in 0..bits {
            let target_bit = bit_index + bit as usize;
            let target_byte = target_bit / 8;
            let intra_byte = target_bit % 8;
            // Each codebook is a power of two with at most 3 bits, so the AND-mask
            // and shift cannot overflow `u8`.
            output[target_byte] |= ((code >> bit) & 1) << intra_byte;
        }
        bit_index += bits as usize;
    }

    Ok(output)
}

/// Dequantize the bitstream into a dense `f32` vector. Positions whose attention
/// weight is below `threshold` are zeroed out instead of being looked up — the
/// "sparse" half of TurboQuant.
pub fn decompress_values_sparse(
    packed: &[u8],
    value_count: usize,
    bits: u8,
    attention: &[f32],
    threshold: f32,
) -> Result<Vec<f32>, TurboQuantError> {
    if attention.len() != value_count {
        return Err(TurboQuantError::AttentionLengthMismatch {
            expected: value_count,
            actual: attention.len(),
        });
    }

    let expected_len = packed_len(value_count, bits)?;
    if packed.len() != expected_len {
        return Err(TurboQuantError::PackedLengthMismatch {
            expected: expected_len,
            actual: packed.len(),
        });
    }

    let levels = levels_for_bits(bits).expect("bit rate already validated above");

    let mut output = vec![0.0f32; value_count];
    for (index, slot) in output.iter_mut().enumerate() {
        if attention[index] < threshold {
            // Sparse: skip the look-up entirely; the slot stays at 0.0.
            continue;
        }
        let code = unpack_code(packed, index, bits);
        debug_assert!(
            (code as usize) < levels.len(),
            "code from a 2|3-bit packed stream cannot exceed the codebook size",
        );
        *slot = levels[code as usize];
    }
    Ok(output)
}

fn unpack_code(packed: &[u8], value_index: usize, bits: u8) -> u8 {
    let bit_index = value_index * bits as usize;
    let byte_index = bit_index / 8;
    let intra_byte = bit_index % 8;
    // Read a 16-bit window straddling the byte boundary so a code of up to 8 bits
    // can be extracted with a single shift+mask. The C++ shim does the same;
    // since `packed_len` always rounds up, the high byte is always in-range when
    // there is a value to read.
    let lo = packed[byte_index] as u16;
    let hi = if byte_index + 1 < packed.len() {
        packed[byte_index + 1] as u16
    } else {
        0
    };
    let window = lo | (hi << 8);
    let mask = (1u16 << bits) - 1;
    ((window >> intra_byte) & mask) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turboquant_round_trips_quantizable_values() {
        let source = vec![
            -1.0f32,
            -0.333_333_34,
            0.333_333_34,
            1.0,
            1.0,
            0.333_333_34,
            -0.333_333_34,
            -1.0,
        ];
        let packed = compress_values(&source, 2).expect("packing should succeed");
        let decoded =
            decompress_values_sparse(&packed, source.len(), 2, &vec![1.0; source.len()], 0.0)
                .expect("decode should succeed");
        assert_eq!(decoded, source);
    }

    #[test]
    fn sparse_decode_zeroes_values_below_threshold() {
        let source = vec![-1.0f32, -0.333_333_34, 0.333_333_34, 1.0];
        let packed = compress_values(&source, 2).expect("packing should succeed");
        let decoded =
            decompress_values_sparse(&packed, source.len(), 2, &[1.0, 0.0, 0.5, 0.0], 0.1)
                .expect("decode should succeed");
        assert_eq!(decoded, vec![-1.0, 0.0, 0.333_333_34, 0.0]);
    }

    #[test]
    fn three_bit_codebook_round_trips() {
        let source: Vec<f32> = LEVELS_3_BIT.to_vec();
        let packed = compress_values(&source, 3).expect("packing should succeed");
        let decoded =
            decompress_values_sparse(&packed, source.len(), 3, &vec![1.0; source.len()], 0.0)
                .expect("decode should succeed");
        assert_eq!(decoded, source);
    }

    #[test]
    fn unsupported_bit_rate_is_rejected() {
        assert!(matches!(
            packed_len(10, 4),
            Err(TurboQuantError::UnsupportedBitRate(4))
        ));
        assert!(matches!(
            compress_values(&[0.0], 4),
            Err(TurboQuantError::UnsupportedBitRate(4))
        ));
    }

    #[test]
    fn attention_length_mismatch_is_reported() {
        let packed = compress_values(&[0.5], 2).expect("packing should succeed");
        let err =
            decompress_values_sparse(&packed, 1, 2, &[], 0.0).expect_err("mismatch should error");
        assert!(matches!(
            err,
            TurboQuantError::AttentionLengthMismatch {
                expected: 1,
                actual: 0,
            }
        ));
    }

    #[test]
    fn packed_length_mismatch_is_reported() {
        let err = decompress_values_sparse(&[0u8; 1], 8, 2, &[1.0; 8], 0.0)
            .expect_err("packed too short should error");
        assert!(matches!(err, TurboQuantError::PackedLengthMismatch { .. }));
    }

    #[test]
    fn packed_length_matches_div_ceil_of_total_bits() {
        // 5 values at 3 bits = 15 bits → 2 bytes (15 div_ceil 8).
        assert_eq!(packed_len(5, 3).expect("5 values at 3 bits should fit"), 2);
        // 8 values at 2 bits = 16 bits → 2 bytes.
        assert_eq!(packed_len(8, 2).expect("8 values at 2 bits should fit"), 2);
        // 0 values → 0 bytes regardless of bit rate.
        assert_eq!(packed_len(0, 2).expect("0 values at 2 bits should fit"), 0);
        assert_eq!(packed_len(0, 3).expect("0 values at 3 bits should fit"), 0);
    }
}
