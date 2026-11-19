//! Локальное шифрование конфиденциальных данных острова (OAuth/CalDAV/изображения).

pub const SEALED_MAGIC: &[u8; 4] = b"ISL1";

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Чистая реализация SHA-256 (FIPS 180-4) без внешних зависимостей.
pub fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut msg = Vec::with_capacity((data.len() + 9 + 63) & !63);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());

    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA256_K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        let deltas = [a, b, c, d, e, f, g, hh];
        for i in 0..8 {
            h[i] = h[i].wrapping_add(deltas[i]);
        }
    }
    let mut out = [0u8; 32];
    for (i, w) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    out
}

fn machine_key() -> [u8; 32] {
    let mut seed = Vec::new();
    for var in ["USER", "USERNAME", "HOME", "USERPROFILE", "COMPUTERNAME", "HOSTNAME"] {
        if let Ok(v) = std::env::var(var) {
            seed.extend_from_slice(v.as_bytes());
        }
    }
    seed.extend_from_slice(b"KiteIsland-LocalSeal-v1");
    sha256_bytes(&seed)
}

fn keystream_xor(key: &[u8; 32], nonce: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for (block_idx, chunk) in data.chunks(32).enumerate() {
        let mut inp = Vec::with_capacity(52);
        inp.extend_from_slice(key);
        inp.extend_from_slice(nonce);
        inp.extend_from_slice(&(block_idx as u32).to_le_bytes());
        let mask = sha256_bytes(&inp);
        for (i, &b) in chunk.iter().enumerate() {
            out.push(b ^ mask[i]);
        }
    }
    out
}

fn compute_tag(key: &[u8; 32], nonce: &[u8; 16], cipher: &[u8]) -> [u8; 16] {
    let mut inp = Vec::with_capacity(key.len() + nonce.len() + cipher.len());
    inp.extend_from_slice(key);
    inp.extend_from_slice(nonce);
    inp.extend_from_slice(cipher);
    let h = sha256_bytes(&inp);
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&h[..16]);
    tag
}

pub fn seal(data: &[u8]) -> Option<Vec<u8>> {
    let key = machine_key();
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut nonce_seed = Vec::with_capacity(24);
    nonce_seed.extend_from_slice(&t.to_le_bytes());
    nonce_seed.extend_from_slice(&std::process::id().to_le_bytes());
    nonce_seed.extend_from_slice(&(data.len() as u32).to_le_bytes());
    let nh = sha256_bytes(&nonce_seed);
    let mut nonce = [0u8; 16];
    nonce.copy_from_slice(&nh[..16]);

    let cipher = keystream_xor(&key, &nonce, data);
    let tag = compute_tag(&key, &nonce, &cipher);

    let mut out = Vec::with_capacity(SEALED_MAGIC.len() + nonce.len() + tag.len() + cipher.len());
    out.extend_from_slice(SEALED_MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&tag);
    out.extend_from_slice(&cipher);
    Some(out)
}

pub fn unseal(data: &[u8]) -> Option<Vec<u8>> {
    if !is_sealed(data) {
        return None;
    }
    let rest = &data[SEALED_MAGIC.len()..];
    if rest.len() < 32 {
        return None;
    }
    let mut nonce = [0u8; 16];
    nonce.copy_from_slice(&rest[..16]);
    let tag = &rest[16..32];
    let cipher = &rest[32..];
    let key = machine_key();
    let expect = compute_tag(&key, &nonce, cipher);
    if expect.ct_eq_slice(tag) {
        Some(keystream_xor(&key, &nonce, cipher))
    } else {
        None
    }
}

trait CtEq {
    fn ct_eq_slice(&self, other: &[u8]) -> bool;
}
impl CtEq for [u8; 16] {
    fn ct_eq_slice(&self, other: &[u8]) -> bool {
        if self.len() != other.len() {
            return false;
        }
        let mut diff = 0u8;
        for (a, b) in self.iter().zip(other.iter()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

pub fn is_sealed(data: &[u8]) -> bool {
    data.starts_with(SEALED_MAGIC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_roundtrip() {
        let data = "секрет: пароль 123".as_bytes();
        let sealed = seal(data).unwrap();
        assert!(is_sealed(&sealed));
        assert_ne!(&sealed[4..], data);
        assert_eq!(unseal(&sealed).as_deref(), Some(data));
    }

    #[test]
    fn unseal_rejects_plain_data() {
        assert!(!is_sealed(b"hello"));
        assert!(unseal(b"hello").is_none());
    }
}
