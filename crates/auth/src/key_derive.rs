//! Key derivation — pure-Rust HMAC-SHA256 based key derivation.
//!
//! Implements HKDF-like key derivation without external crypto dependencies.
//! The SHA-256 and HMAC primitives are implemented inline following FIPS 180-4
//! and RFC 2104.

use std::fmt;

// ── Configuration ──────────────────────────────────────────────────────

/// Configuration for key derivation.
#[derive(Debug, Clone)]
pub struct KeyDerivationConfig {
    /// Context string mixed into derivation (e.g. "encryption", "signing").
    pub context: String,
    /// Output key length in bytes (max 32 for SHA-256).
    pub output_len: usize,
}

impl Default for KeyDerivationConfig {
    fn default() -> Self {
        Self {
            context: String::new(),
            output_len: 32,
        }
    }
}

// ── Derived key ────────────────────────────────────────────────────────

/// A derived key that zeroes its memory on drop.
pub struct DerivedKey {
    bytes: Vec<u8>,
}

impl DerivedKey {
    fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Raw key bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Key length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the key is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Hex-encoded representation.
    #[must_use]
    pub fn to_hex(&self) -> String {
        use std::fmt::Write;
        self.bytes
            .iter()
            .fold(String::with_capacity(self.bytes.len() * 2), |mut acc, b| {
                let _ = write!(acc, "{b:02x}");
                acc
            })
    }
}

impl Drop for DerivedKey {
    fn drop(&mut self) {
        // Zeroize on drop to limit key exposure in memory.
        // Use `fill` + `black_box` fence to discourage the optimizer from
        // eliding the write. Not a cryptographic guarantee, but best-effort
        // without unsafe or external crates.
        self.bytes.fill(0);
        std::hint::black_box(&self.bytes);
    }
}

impl fmt::Debug for DerivedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DerivedKey([redacted])")
    }
}

// ── Public API ─────────────────────────────────────────────────────────

/// Derive a key from input keying material, salt, and info context.
///
/// Uses HKDF-Extract + HKDF-Expand (RFC 5869) with HMAC-SHA256.
#[must_use]
pub fn derive_key(ikm: &[u8], salt: &[u8], info: &[u8], output_len: usize) -> DerivedKey {
    let len = output_len.min(32);
    // Extract
    let prk = hmac_sha256(salt, ikm);
    // Expand (single block — max 32 bytes)
    let mut expand_input = Vec::with_capacity(info.len() + 1);
    expand_input.extend_from_slice(info);
    expand_input.push(1);
    let okm = hmac_sha256(&prk, &expand_input);
    DerivedKey::new(okm[..len].to_vec())
}

/// Derive an encryption key from a master secret and context.
#[must_use]
pub fn derive_encryption_key(master: &[u8], context: &str) -> DerivedKey {
    let info = format!("crab-code-enc:{context}");
    derive_key(master, b"crab-code-encryption", info.as_bytes(), 32)
}

/// Derive a signing / HMAC key from a master secret and context.
#[must_use]
pub fn derive_signing_key(master: &[u8], context: &str) -> DerivedKey {
    let info = format!("crab-code-sig:{context}");
    derive_key(master, b"crab-code-signing", info.as_bytes(), 32)
}

/// Derive a key using a [`KeyDerivationConfig`].
#[must_use]
pub fn derive_with_config(master: &[u8], salt: &[u8], config: &KeyDerivationConfig) -> DerivedKey {
    derive_key(master, salt, config.context.as_bytes(), config.output_len)
}

/// Compute raw HMAC-SHA256 (exposed for testing / advanced use).
#[must_use]
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    hmac_sha256_impl(key, data)
}

// ── SHA-256 (FIPS 180-4) ──────────────────────────────────────────────

const SHA256_K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

const SHA256_INIT: [u32; 8] = [
    0x6a09_e667,
    0xbb67_ae85,
    0x3c6e_f372,
    0xa54f_f53a,
    0x510e_527f,
    0x9b05_688c,
    0x1f83_d9ab,
    0x5be0_cd19,
];

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut state = SHA256_INIT;
    #[allow(clippy::cast_possible_truncation)] // data.len() won't exceed u64 on any real platform
    let bit_len = (data.len() as u64).wrapping_mul(8);

    // Pad: append 0x80, zeros, then 64-bit big-endian length
    let mut padded = data.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    // Process 512-bit blocks
    for block in padded.chunks_exact(64) {
        sha256_compress(&mut state, block);
    }

    let mut out = [0u8; 32];
    for (idx, word) in state.iter().enumerate() {
        out[idx * 4..idx * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[allow(clippy::many_single_char_names)] // SHA-256 spec uses canonical a–h names
fn sha256_compress(state: &mut [u32; 8], block: &[u8]) {
    let mut w = [0u32; 64];
    for idx in 0..16 {
        w[idx] = u32::from_be_bytes([
            block[idx * 4],
            block[idx * 4 + 1],
            block[idx * 4 + 2],
            block[idx * 4 + 3],
        ]);
    }
    for idx in 16..64 {
        let s0 = w[idx - 15].rotate_right(7) ^ w[idx - 15].rotate_right(18) ^ (w[idx - 15] >> 3);
        let s1 = w[idx - 2].rotate_right(17) ^ w[idx - 2].rotate_right(19) ^ (w[idx - 2] >> 10);
        w[idx] = w[idx - 16]
            .wrapping_add(s0)
            .wrapping_add(w[idx - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;

    for idx in 0..64 {
        let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(sum1)
            .wrapping_add(ch)
            .wrapping_add(SHA256_K[idx])
            .wrapping_add(w[idx]);
        let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = sum0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

// ── HMAC-SHA256 (RFC 2104) ─────────────────────────────────────────────

fn hmac_sha256_impl(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;

    // If key > block size, hash it first
    let key_block = if key.len() > BLOCK_SIZE {
        let h = sha256(key);
        let mut kb = [0u8; BLOCK_SIZE];
        kb[..32].copy_from_slice(&h);
        kb
    } else {
        let mut kb = [0u8; BLOCK_SIZE];
        kb[..key.len()].copy_from_slice(key);
        kb
    };

    // Inner pad
    let mut ipad = [0x36u8; BLOCK_SIZE];
    for (idx, byte) in ipad.iter_mut().enumerate() {
        *byte ^= key_block[idx];
    }

    // Outer pad
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for (idx, byte) in opad.iter_mut().enumerate() {
        *byte ^= key_block[idx];
    }

    // inner = SHA256(ipad || data)
    let mut inner_input = Vec::with_capacity(BLOCK_SIZE + data.len());
    inner_input.extend_from_slice(&ipad);
    inner_input.extend_from_slice(data);
    let inner_hash = sha256(&inner_input);

    // outer = SHA256(opad || inner)
    let mut outer_input = Vec::with_capacity(BLOCK_SIZE + 32);
    outer_input.extend_from_slice(&opad);
    outer_input.extend_from_slice(&inner_hash);
    sha256(&outer_input)
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── SHA-256 test vectors (NIST) ────────────────────────────────────

    #[test]
    fn sha256_empty() {
        let hash = sha256(b"");
        assert_eq!(
            hex(&hash),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_abc() {
        let hash = sha256(b"abc");
        assert_eq!(
            hex(&hash),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_long() {
        let hash = sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq");
        assert_eq!(
            hex(&hash),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    // ── HMAC-SHA256 test vectors (RFC 4231) ────────────────────────────

    #[test]
    fn hmac_sha256_rfc4231_case1() {
        // Key = 0x0b repeated 20 times, Data = "Hi There"
        let key = vec![0x0b; 20];
        let mac = hmac_sha256(&key, b"Hi There");
        assert_eq!(
            hex(&mac),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn hmac_sha256_rfc4231_case2() {
        // Key = "Jefe", Data = "what do ya want for nothing?"
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    // ── Key derivation ─────────────────────────────────────────────────

    #[test]
    fn derive_key_deterministic() {
        let k1 = derive_key(b"secret", b"salt", b"info", 32);
        let k2 = derive_key(b"secret", b"salt", b"info", 32);
        assert_eq!(k1.as_bytes(), k2.as_bytes());
    }

    #[test]
    fn derive_key_different_salt_different_output() {
        let k1 = derive_key(b"secret", b"salt-a", b"info", 32);
        let k2 = derive_key(b"secret", b"salt-b", b"info", 32);
        assert_ne!(k1.as_bytes(), k2.as_bytes());
    }

    #[test]
    fn derive_key_different_info_different_output() {
        let k1 = derive_key(b"secret", b"salt", b"enc", 32);
        let k2 = derive_key(b"secret", b"salt", b"sig", 32);
        assert_ne!(k1.as_bytes(), k2.as_bytes());
    }

    #[test]
    fn derive_key_truncated_output() {
        let k = derive_key(b"secret", b"salt", b"info", 16);
        assert_eq!(k.len(), 16);
    }

    #[test]
    fn derive_key_clamped_to_32() {
        let k = derive_key(b"secret", b"salt", b"info", 64);
        assert_eq!(k.len(), 32);
    }

    #[test]
    fn derive_encryption_key_works() {
        let k = derive_encryption_key(b"master-secret", "session-1");
        assert_eq!(k.len(), 32);
        assert!(!k.is_empty());
    }

    #[test]
    fn derive_signing_key_works() {
        let k = derive_signing_key(b"master-secret", "session-1");
        assert_eq!(k.len(), 32);
    }

    #[test]
    fn encryption_and_signing_keys_differ() {
        let enc = derive_encryption_key(b"master", "ctx");
        let sig = derive_signing_key(b"master", "ctx");
        assert_ne!(enc.as_bytes(), sig.as_bytes());
    }

    #[test]
    fn derive_with_config_works() {
        let config = KeyDerivationConfig {
            context: "test-context".into(),
            output_len: 24,
        };
        let k = derive_with_config(b"master", b"salt", &config);
        assert_eq!(k.len(), 24);
    }

    #[test]
    fn default_config() {
        let config = KeyDerivationConfig::default();
        assert!(config.context.is_empty());
        assert_eq!(config.output_len, 32);
    }

    // ── DerivedKey ─────────────────────────────────────────────────────

    #[test]
    fn derived_key_hex() {
        let k = derive_key(b"test", b"salt", b"info", 4);
        let h = k.to_hex();
        assert_eq!(h.len(), 8); // 4 bytes * 2 hex chars
    }

    #[test]
    fn derived_key_debug_redacted() {
        let k = derive_key(b"test", b"salt", b"info", 32);
        let dbg = format!("{k:?}");
        assert_eq!(dbg, "DerivedKey([redacted])");
        assert!(!dbg.contains("test"));
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
