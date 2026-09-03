//! SHA-256 for every digest the replay side takes, over the `sha2` crate.
//!
//! The hand-rolled compression function this module used to carry ran at
//! roughly 200 MB/s; a finalize of rc_vanilla8 hashes 29 GB of canonical world
//! text twice (once at emit, once at parity) and spent 250 s in it. `sha2`
//! selects the SHA-NI instructions at run time where the CPU has them and
//! falls back to portable code where it does not; the digest is the same
//! function either way, so no recorded hash changes.

use sha2::Digest as _;

#[derive(Clone)]
pub struct Sha256 {
    inner: sha2::Sha256,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Self {
        Sha256 {
            inner: sha2::Sha256::new(),
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    pub fn finalize(self) -> [u8; 32] {
        self.inner.finalize().into()
    }

    pub fn digest(data: &[u8]) -> [u8; 32] {
        sha2::Sha256::digest(data).into()
    }

    pub fn hex(data: &[u8]) -> String {
        to_hex(&Sha256::digest(data))
    }
}

pub fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(
            Sha256::hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            Sha256::hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            Sha256::hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn streaming_matches_oneshot() {
        let data: Vec<u8> = (0u8..=255).cycle().take(1000).collect();
        let one = Sha256::hex(&data);
        let mut h = Sha256::new();
        for chunk in data.chunks(7) {
            h.update(chunk);
        }
        assert_eq!(to_hex(&h.finalize()), one);
    }

    #[test]
    fn block_boundary_lengths() {
        for n in [0usize, 1, 55, 56, 57, 63, 64, 65, 119, 120, 121] {
            let data = vec![0xa5u8; n];
            let mut h = Sha256::new();
            h.update(&data);
            let streamed = to_hex(&h.finalize());
            assert_eq!(streamed, Sha256::hex(&data), "n={n}");
        }
    }
}
