//! The fingerprint `tools/sites.py` computes: the length, then little-endian 8-byte words with
//! every masked 4-byte field zeroed and the tail padded with zeros.

const M1: u64 = 0x9E37_79B9_7F4A_7C15;
const M2: u64 = 0xBF58_476D_1CE4_E5B9;
const M3: u64 = 0x94D0_49BB_1331_11EB;

pub(crate) fn fingerprint(bytes: &[u8], mask: &[u32]) -> u64 {
    let mut h = (bytes.len() as u64).wrapping_mul(M1);
    let mut fields = mask.iter().map(|&k| k as usize).peekable();
    let mut chunks = bytes.chunks(8);
    let mut offset = 0;
    for chunk in &mut chunks {
        let mut word = [0; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        // A field may start in the previous word and spill into this one.
        while let Some(&k) = fields.peek() {
            if k >= offset + 8 {
                break;
            }
            for byte in k.max(offset)..(k + 4).min(offset + 8) {
                word[byte - offset] = 0;
            }
            if k + 4 > offset + 8 {
                break;
            }
            fields.next();
        }
        h = (h ^ u64::from_le_bytes(word)).wrapping_mul(M2);
        h ^= h >> 29;
        offset += 8;
    }
    h ^= h >> 32;
    h = h.wrapping_mul(M3);
    h ^ (h >> 29)
}

/// Bytes the prefilter covers before a full fingerprint is computed.
const HEAD: usize = 16;

/// The prefilter: the fingerprint of the first 16 bytes (or fewer) with masked bytes zeroed.
pub(crate) fn head(bytes: &[u8], mask: &[u32]) -> u64 {
    let n = bytes.len().min(HEAD);
    let mut h = [0; HEAD];
    h[..n].copy_from_slice(&bytes[..n]);
    for &k in mask {
        let k = k as usize;
        if k >= n {
            break;
        }
        h[k..(k + 4).min(n)].fill(0);
    }
    fingerprint(&h[..n], &[])
}

#[cfg(test)]
mod tests {
    use super::fingerprint;

    #[test]
    fn masks_fields_across_words() {
        let a = [1_u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let mut b = a;
        b[6..10].copy_from_slice(&[0xaa; 4]);
        assert_eq!(fingerprint(&a, &[6]), fingerprint(&b, &[6]));
        assert_ne!(fingerprint(&a, &[]), fingerprint(&b, &[]));
        assert_ne!(fingerprint(&a, &[6]), fingerprint(&a[..11], &[6]));
    }
}
