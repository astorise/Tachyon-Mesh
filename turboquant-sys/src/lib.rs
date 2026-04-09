use std::{error::Error, fmt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurboQuantError {
    UnsupportedBitRate(u8),
    AttentionLengthMismatch { expected: usize, actual: usize },
    PackedLengthMismatch { expected: usize, actual: usize },
    FfiFailure(i32),
}

impl fmt::Display for TurboQuantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedBitRate(bits) => {
                write!(f, "TurboQuant mock only supports 2-bit or 3-bit packing, got {bits}")
            }
            Self::AttentionLengthMismatch { expected, actual } => write!(
                f,
                "TurboQuant sparse decode expects {expected} attention weights but received {actual}"
            ),
            Self::PackedLengthMismatch { expected, actual } => write!(
                f,
                "TurboQuant packed payload should be {expected} bytes but received {actual}"
            ),
            Self::FfiFailure(code) => write!(f, "TurboQuant native shim failed with code {code}"),
        }
    }
}

impl Error for TurboQuantError {}

unsafe extern "C" {
    fn tq_packed_len(value_count: usize, bits: u8) -> usize;
    fn tq_compress_values_f32(
        input: *const f32,
        value_count: usize,
        bits: u8,
        output: *mut u8,
        output_len: usize,
    ) -> i32;
    fn tq_decompress_values_sparse_f32(
        packed: *const u8,
        packed_len: usize,
        value_count: usize,
        bits: u8,
        attention: *const f32,
        attention_len: usize,
        threshold: f32,
        output: *mut f32,
        output_len: usize,
    ) -> i32;
}

fn ensure_supported_bits(bits: u8) -> Result<(), TurboQuantError> {
    match bits {
        2 | 3 => Ok(()),
        _ => Err(TurboQuantError::UnsupportedBitRate(bits)),
    }
}

pub fn packed_len(value_count: usize, bits: u8) -> Result<usize, TurboQuantError> {
    ensure_supported_bits(bits)?;
    Ok(unsafe { tq_packed_len(value_count, bits) })
}

pub fn compress_values(input: &[f32], bits: u8) -> Result<Vec<u8>, TurboQuantError> {
    let expected_len = packed_len(input.len(), bits)?;
    let mut output = vec![0u8; expected_len];
    let status = unsafe {
        tq_compress_values_f32(
            input.as_ptr(),
            input.len(),
            bits,
            output.as_mut_ptr(),
            output.len(),
        )
    };
    if status == 0 {
        Ok(output)
    } else {
        Err(TurboQuantError::FfiFailure(status))
    }
}

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

    let mut output = vec![0.0f32; value_count];
    let status = unsafe {
        tq_decompress_values_sparse_f32(
            packed.as_ptr(),
            packed.len(),
            value_count,
            bits,
            attention.as_ptr(),
            attention.len(),
            threshold,
            output.as_mut_ptr(),
            output.len(),
        )
    };
    if status == 0 {
        Ok(output)
    } else {
        Err(TurboQuantError::FfiFailure(status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turboquant_mock_round_trips_quantizable_values() {
        let source = vec![
            -1.0f32,
            -0.33333334f32,
            0.33333334f32,
            1.0f32,
            1.0f32,
            0.33333334f32,
            -0.33333334f32,
            -1.0f32,
        ];
        let packed = compress_values(&source, 2).expect("packing should succeed");
        let decoded =
            decompress_values_sparse(&packed, source.len(), 2, &vec![1.0; source.len()], 0.0)
                .expect("decode should succeed");

        assert_eq!(decoded, source);
    }

    #[test]
    fn sparse_decode_zeroes_values_below_threshold() {
        let source = vec![-1.0f32, -0.33333334f32, 0.33333334f32, 1.0f32];
        let packed = compress_values(&source, 2).expect("packing should succeed");
        let decoded =
            decompress_values_sparse(&packed, source.len(), 2, &[1.0, 0.0, 0.5, 0.0], 0.1)
                .expect("decode should succeed");

        assert_eq!(decoded, vec![-1.0, 0.0, 0.33333334, 0.0]);
    }
}
