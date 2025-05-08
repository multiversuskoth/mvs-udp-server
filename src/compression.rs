/// Module implementing the zero-suppression bitmask compression algorithm
/// as ported from the TypeScript implementation.

/// Maximum buffer size for compression/decompression
const MAX_BUFFER_SIZE: usize = 1024;

/// Compresses a byte slice using an 8‑byte zero‑suppression bitmask algorithm,
/// writing into a buffer with maximum size of 1024 bytes.
///
/// # Arguments
///
/// * `input` - The data to compress
///
/// # Returns
///
/// A `Vec<u8>` containing the compressed data (≤1024 bytes).
///
/// # Errors
///
/// Returns an error if the compressed output would exceed 1024 bytes.
pub fn compress_packet(input: &[u8]) -> Result<Vec<u8>, String> {
    let n = input.len();
    if n == 0 {
        return Ok(Vec::new());
    }

    // Pre-allocate buffer with max size
    let mut out_buf = Vec::with_capacity(MAX_BUFFER_SIZE);
    let mut in_pos = 0;

    while in_pos < n {
        // Check if we have enough space for mask byte
        if out_buf.len() >= MAX_BUFFER_SIZE {
            return Err("compress_packet: output buffer overflow (1024 bytes)".to_string());
        }

        // Reserve spot for mask byte
        let mask_pos = out_buf.len();
        out_buf.push(0); // Will be updated with the mask later
        let mut mask = 0u8;

        // Process up to 8 bytes (one mask's worth)
        for bit in 0..8 {
            if in_pos >= n {
                break;
            }

            let v = input[in_pos];
            in_pos += 1;

            if v != 0 {
                mask |= 1 << bit;
                
                // Ensure we have space for this non-zero byte
                if out_buf.len() >= MAX_BUFFER_SIZE {
                    return Err("compress_packet: output buffer overflow (1024 bytes)".to_string());
                }
                
                out_buf.push(v);
            }
        }

        // Update the mask byte we reserved earlier
        out_buf[mask_pos] = mask;
    }

    Ok(out_buf)
}

/// Decompresses a byte slice that was compressed with the zero‑suppression bitmask algorithm.
///
/// # Arguments
///
/// * `compressed_buffer` - The compressed input (mask + non‑zero bytes)
/// * `original_length` - The expected length of the decompressed data.
///   If the decompressed data would exceed originalLength, it's truncated;
///   if originalLength > 1024, an error is returned.
///
/// # Returns
///
/// A `Vec<u8>` containing the decompressed data (≤ originalLength ≤ 1024).
///
/// # Errors
///
/// Returns an error if the compressed data is malformed or the decompressed output would exceed 1024 bytes.
pub fn decompress_packet(compressed_buffer: &[u8], original_length: Option<usize>) -> Result<Vec<u8>, String> {
    let original_len = original_length.unwrap_or(MAX_BUFFER_SIZE);
    
    if original_len > MAX_BUFFER_SIZE {
        return Err(format!("decompress_packet: originalLength ({}) must be between 0 and 1024", original_len));
    }

    // Pre-allocate output buffer of the requested size
    let mut out_buf = vec![0u8; original_len];
    let mut read_pos = 0;
    let mut write_pos = 0;

    while read_pos < compressed_buffer.len() && write_pos < original_len {
        // Read the mask byte
        if read_pos >= compressed_buffer.len() {
            return Err("decompress_packet: truncated compressed data".to_string());
        }
        
        let mask = compressed_buffer[read_pos];
        read_pos += 1;

        // Process all bits in the mask
        for bit in 0..8 {
            if write_pos >= original_len {
                // We've reached our target size, we're done
                break;
            }
            
            let is_non_zero = (mask & (1 << bit)) != 0;
            
            if is_non_zero {
                if read_pos >= compressed_buffer.len() {
                    return Err("decompress_packet: truncated compressed data".to_string());
                }
                
                out_buf[write_pos] = compressed_buffer[read_pos];
                read_pos += 1;
            } else {
                // For zero bits, we just leave the buffer's 0 value
            }
            
            write_pos += 1;
        }
    }

    // Return only the filled portion (up to original_len)
    out_buf.truncate(write_pos);
    Ok(out_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_decompress_empty() {
        let empty: Vec<u8> = vec![];
        let compressed = compress_packet(&empty).unwrap();
        assert_eq!(compressed.len(), 0);
        
        let decompressed = decompress_packet(&compressed, Some(0)).unwrap();
        assert_eq!(decompressed.len(), 0);
    }

    #[test]
    fn test_compress_decompress_simple() {
        let input = vec![1, 0, 3, 0, 0, 6, 7, 0, 9];
        let compressed = compress_packet(&input).unwrap();
        let decompressed = decompress_packet(&compressed, Some(input.len())).unwrap();
        
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_compress_decompress_all_zeros() {
        let input = vec![0, 0, 0, 0, 0, 0, 0, 0];
        let compressed = compress_packet(&input).unwrap();
        
        // For 8 zeros, we should just have a single mask byte of 0
        assert_eq!(compressed.len(), 1);
        assert_eq!(compressed[0], 0);
        
        let decompressed = decompress_packet(&compressed, Some(input.len())).unwrap();
        assert_eq!(decompressed, input);
    }

    #[test]
    fn test_compress_decompress_all_nonzeros() {
        let input = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let compressed = compress_packet(&input).unwrap();
        
        // For 8 non-zeros, we should have 1 mask byte + 8 data bytes
        assert_eq!(compressed.len(), 9);
        assert_eq!(compressed[0], 0xFF); // all bits set
        
        let decompressed = decompress_packet(&compressed, Some(input.len())).unwrap();
        assert_eq!(decompressed, input);
    }
}