//! The standard security handler (ISO 32000-2, 7.6.4): files protected
//! with a password, or only with permissions (opened with an empty
//! password, as many government forms are). Strings and streams are
//! decrypted as objects are read, and encrypted again when objects are
//! written, with the file's own key, so an update stays protected the same
//! way.

use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use md5::{Digest, Md5};
use sha2::{Sha256, Sha384, Sha512};

use super::object::{Dict, Obj, Ref};

const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08, 0x2E, 0x2E, 0x00,
    0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    None,
    Rc4,
    Aes128,
    Aes256,
}

/// What it takes to decrypt and encrypt a file's objects.
#[derive(Debug, Clone)]
pub struct Crypt {
    key: Vec<u8>,
    strings: Method,
    streams: Method,
    /// The encryption dictionary's own object (never encrypted).
    pub dict_obj: Option<Ref>,
    encrypt_metadata: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CryptError {
    /// The password does not open the file.
    Password,
    /// A security handler or method edits does not know.
    Unsupported(String),
}

fn bytes(d: &Dict, k: &[u8]) -> Vec<u8> {
    match d.get(k) {
        Some(Obj::Str(s)) => s.clone(),
        _ => Vec::new(),
    }
}

fn int(d: &Dict, k: &[u8]) -> i64 {
    d.get(k).and_then(Obj::as_int).unwrap_or(0)
}

impl Crypt {
    /// Opens the encryption dictionary `enc` with `password` (empty for
    /// files that only restrict permissions). `id0` is the first file ID.
    pub fn new(enc: &Dict, dict_obj: Option<Ref>, id0: &[u8], password: &[u8]) -> Result<Crypt, CryptError> {
        if enc.name(b"Filter") != Some(b"Standard") {
            return Err(CryptError::Unsupported("a security handler other than the standard one".into()));
        }
        let v = int(enc, b"V");
        let r = int(enc, b"R");
        let method_of = |name: Option<&[u8]>| -> Result<Method, CryptError> {
            match name {
                None | Some(b"Identity") => Ok(Method::None),
                Some(n) => {
                    let cf = enc.get(b"CF").and_then(Obj::as_dict).and_then(|cf| cf.get(n)).and_then(Obj::as_dict);
                    match cf.and_then(|c| c.name(b"CFM")) {
                        Some(b"V2") => Ok(Method::Rc4),
                        Some(b"AESV2") => Ok(Method::Aes128),
                        Some(b"AESV3") => Ok(Method::Aes256),
                        Some(b"None") | None => Ok(Method::None),
                        Some(o) => Err(CryptError::Unsupported(String::from_utf8_lossy(o).into_owned())),
                    }
                }
            }
        };
        let (strings, streams) = match v {
            1 | 2 => (Method::Rc4, Method::Rc4),
            4 | 5 => (method_of(enc.name(b"StrF"))?, method_of(enc.name(b"StmF"))?),
            _ => return Err(CryptError::Unsupported(format!("encryption version {v}"))),
        };
        let encrypt_metadata = !matches!(enc.get(b"EncryptMetadata"), Some(Obj::Bool(false)));
        let o = bytes(enc, b"O");
        let u = bytes(enc, b"U");
        let key = if r >= 5 {
            key_r6(enc, password, r).ok_or(CryptError::Password)?
        } else {
            let len = match v {
                1 => 5,
                _ => (int(enc, b"Length").clamp(40, 128) / 8) as usize,
            };
            let p = int(enc, b"P") as i32;
            let key = key_r4(password, &o, p, id0, r, len, encrypt_metadata);
            // Check the key against /U.
            let ok = if r == 2 {
                rc4(&key, &PAD) == u.get(..32).unwrap_or(&[])
            } else {
                let mut h = Md5::new();
                h.update(PAD);
                h.update(id0);
                let mut x = rc4(&key, &h.finalize());
                for i in 1..=19u8 {
                    let k: Vec<u8> = key.iter().map(|b| b ^ i).collect();
                    x = rc4(&k, &x);
                }
                u.len() >= 16 && x[..] == u[..16]
            };
            if !ok {
                return Err(CryptError::Password);
            }
            key
        };
        Ok(Crypt { key, strings, streams, dict_obj, encrypt_metadata })
    }

    fn object_key(&self, m: Method, r: Ref) -> Vec<u8> {
        if m == Method::Aes256 {
            return self.key.clone();
        }
        let mut h = Md5::new();
        h.update(&self.key);
        h.update(&r.0.to_le_bytes()[..3]);
        h.update(r.1.to_le_bytes());
        if m == Method::Aes128 {
            h.update(b"sAlT");
        }
        let n = (self.key.len() + 5).min(16);
        h.finalize()[..n].to_vec()
    }

    fn decrypt_with(&self, m: Method, r: Ref, data: &[u8]) -> Vec<u8> {
        match m {
            Method::None => data.to_vec(),
            Method::Rc4 => rc4(&self.object_key(m, r), data),
            Method::Aes128 | Method::Aes256 => aes_cbc_decrypt(&self.object_key(m, r), data).unwrap_or_default(),
        }
    }

    fn encrypt_with(&self, m: Method, r: Ref, data: &[u8]) -> Vec<u8> {
        match m {
            Method::None => data.to_vec(),
            Method::Rc4 => rc4(&self.object_key(m, r), data),
            Method::Aes128 | Method::Aes256 => {
                let key = self.object_key(m, r);
                // A fixed IV per object and content: rebuilding an update gives the same bytes.
                let mut h = Md5::new();
                h.update(&key);
                h.update(data);
                let iv: [u8; 16] = h.finalize().into();
                aes_cbc_encrypt(&key, &iv, data)
            }
        }
    }

    /// `o`, object `r` as read from the file, decrypted.
    pub fn decrypt(&self, r: Ref, o: Obj) -> Obj {
        if Some(r) == self.dict_obj {
            return o;
        }
        self.walk(r, o, false)
    }

    /// `o`, to be written as object `r`, encrypted.
    pub fn encrypt(&self, r: Ref, o: Obj) -> Obj {
        if Some(r) == self.dict_obj {
            return o;
        }
        self.walk(r, o, true)
    }

    /// Like `encrypt`, for a file written anew, where object numbers change
    /// and the caller leaves the encryption dictionary itself alone.
    pub fn encrypt_renumbered(&self, r: Ref, o: Obj) -> Obj {
        self.walk(r, o, true)
    }

    fn walk(&self, r: Ref, o: Obj, enc: bool) -> Obj {
        let f = |m, data: &[u8]| if enc { self.encrypt_with(m, r, data) } else { self.decrypt_with(m, r, data) };
        match o {
            Obj::Str(s) => Obj::Str(f(self.strings, &s)),
            Obj::Array(a) => Obj::Array(a.into_iter().map(|v| self.walk(r, v, enc)).collect()),
            Obj::Dict(d) => Obj::Dict(self.walk_dict(r, d, enc)),
            Obj::Stream(d, data) => {
                // Cross-reference streams are never encrypted; metadata only if asked.
                let skip = d.name(b"Type") == Some(b"XRef")
                    || (!self.encrypt_metadata && d.name(b"Type") == Some(b"Metadata"))
                    || has_identity_crypt(&d);
                let data = if skip { data } else { f(self.streams, &data) };
                Obj::Stream(self.walk_dict(r, d, enc), data)
            }
            other => other,
        }
    }

    fn walk_dict(&self, r: Ref, d: Dict, enc: bool) -> Dict {
        // A signature's /Contents is never encrypted.
        let sig = d.name(b"Type") == Some(b"Sig");
        Dict(
            d.0.into_iter()
                .map(|(k, v)| {
                    if sig && k == b"Contents" {
                        (k, v)
                    } else {
                        let v = self.walk(r, v, enc);
                        (k, v)
                    }
                })
                .collect(),
        )
    }
}

fn has_identity_crypt(d: &Dict) -> bool {
    match d.get(b"Filter") {
        Some(Obj::Name(n)) => n == b"Crypt",
        Some(Obj::Array(a)) => a.first() == Some(&Obj::name("Crypt")),
        _ => false,
    }
}

/// Algorithm 2: the file key for revisions 2 to 4.
fn key_r4(password: &[u8], o: &[u8], p: i32, id0: &[u8], r: i64, len: usize, encrypt_metadata: bool) -> Vec<u8> {
    let mut padded = password.iter().take(32).copied().collect::<Vec<u8>>();
    padded.extend_from_slice(&PAD[..32 - padded.len()]);
    let mut h = Md5::new();
    h.update(&padded);
    h.update(o.get(..32).unwrap_or(o));
    h.update(p.to_le_bytes());
    h.update(id0);
    if r >= 4 && !encrypt_metadata {
        h.update([0xFF; 4]);
    }
    let mut hash = h.finalize().to_vec();
    if r >= 3 {
        for _ in 0..50 {
            hash = Md5::digest(&hash[..len]).to_vec();
        }
    }
    hash.truncate(len);
    hash
}

/// Algorithms 2.A and 2.B: the file key for revisions 5 and 6, from the
/// user password (an owner password would need /O; edits opens files as a
/// reader does).
fn key_r6(enc: &Dict, password: &[u8], r: i64) -> Option<Vec<u8>> {
    let u = bytes(enc, b"U");
    let ue = bytes(enc, b"UE");
    if u.len() < 48 || ue.len() < 32 {
        return None;
    }
    let pw = &password[..password.len().min(127)];
    let hash = |salt: &[u8], extra: &[u8]| if r == 5 { sha256(&[pw, salt, extra]) } else { hash_r6(pw, salt, extra) };
    if hash(&u[32..40], &[]) != u[..32] {
        return None;
    }
    let k = hash(&u[40..48], &[]);
    // UE decrypted with AES-256, no IV (zero), no padding.
    let mut out = ue[..32].to_vec();
    aes_cbc_decrypt_raw(&k, &[0u8; 16], &mut out);
    Some(out)
}

fn sha256(parts: &[&[u8]]) -> Vec<u8> {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().to_vec()
}

/// Algorithm 2.B (revision 6).
fn hash_r6(pw: &[u8], salt: &[u8], extra: &[u8]) -> Vec<u8> {
    let mut k = sha256(&[pw, salt, extra]);
    let mut round = 0u32;
    loop {
        let mut k1 = Vec::with_capacity(64 * (pw.len() + k.len() + extra.len()));
        for _ in 0..64 {
            k1.extend_from_slice(pw);
            k1.extend_from_slice(&k);
            k1.extend_from_slice(extra);
        }
        let key: [u8; 16] = k[..16].try_into().expect("16 bytes");
        let iv: [u8; 16] = k[16..32].try_into().expect("16 bytes");
        aes128_cbc_encrypt_raw(&key, &iv, &mut k1);
        let e = k1;
        let sum: u32 = e[..16].iter().map(|v| *v as u32).sum();
        k = match sum % 3 {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };
        round += 1;
        if round >= 64 && (*e.last().unwrap_or(&0) as u32) + 32 <= round {
            break;
        }
    }
    k.truncate(32);
    k
}

pub fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0u8;
    if !key.is_empty() {
        for i in 0..256 {
            j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
            s.swap(i, j as usize);
        }
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            b ^ s[s[i as usize].wrapping_add(s[j as usize]) as usize]
        })
        .collect()
}

enum Aes {
    K128(Box<aes::Aes128>),
    K256(Box<aes::Aes256>),
}

impl Aes {
    fn new(key: &[u8]) -> Option<Aes> {
        match key.len() {
            16 => aes::Aes128::new_from_slice(key).ok().map(|c| Aes::K128(Box::new(c))),
            32 => aes::Aes256::new_from_slice(key).ok().map(|c| Aes::K256(Box::new(c))),
            _ => None,
        }
    }

    fn encrypt(&self, b: &mut [u8; 16]) {
        let block = b.into();
        match self {
            Aes::K128(c) => c.encrypt_block(block),
            Aes::K256(c) => c.encrypt_block(block),
        }
    }

    fn decrypt(&self, b: &mut [u8; 16]) {
        let block = b.into();
        match self {
            Aes::K128(c) => c.decrypt_block(block),
            Aes::K256(c) => c.decrypt_block(block),
        }
    }
}

fn aes128_cbc_encrypt_raw(key: &[u8; 16], iv: &[u8; 16], data: &mut [u8]) {
    let Some(c) = Aes::new(key) else { return };
    let mut prev = *iv;
    for chunk in data.as_chunks_mut::<16>().0 {
        for (b, p) in chunk.iter_mut().zip(prev) {
            *b ^= p;
        }
        c.encrypt(chunk);
        prev = *chunk;
    }
}

fn aes_cbc_decrypt_raw(key: &[u8], iv: &[u8; 16], data: &mut [u8]) {
    let Some(c) = Aes::new(key) else { return };
    let mut prev = *iv;
    for chunk in data.as_chunks_mut::<16>().0 {
        let cipher = *chunk;
        c.decrypt(chunk);
        for (b, p) in chunk.iter_mut().zip(prev) {
            *b ^= p;
        }
        prev = cipher;
    }
}

/// IV first, then PKCS#5-padded blocks.
fn aes_cbc_decrypt(key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 32 || !data.len().is_multiple_of(16) {
        // An empty string encrypts to the IV alone, or files carry junk.
        return (data.len() <= 16).then(Vec::new);
    }
    let iv: [u8; 16] = data[..16].try_into().ok()?;
    let mut out = data[16..].to_vec();
    aes_cbc_decrypt_raw(key, &iv, &mut out);
    let n = *out.last()? as usize;
    if (1..=16).contains(&n) && out.len() >= n {
        out.truncate(out.len() - n);
    }
    Some(out)
}

fn aes_cbc_encrypt(key: &[u8], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let Some(c) = Aes::new(key) else { return data.to_vec() };
    let n = 16 - data.len() % 16;
    let mut buf = data.to_vec();
    buf.extend(std::iter::repeat_n(n as u8, n));
    let mut prev = *iv;
    for chunk in buf.as_chunks_mut::<16>().0 {
        for (b, p) in chunk.iter_mut().zip(prev) {
            *b ^= p;
        }
        c.encrypt(chunk);
        prev = *chunk;
    }
    let mut out = iv.to_vec();
    out.extend_from_slice(&buf);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc4_matches_a_known_vector() {
        // "Key" / "Plaintext" from the RC4 test vectors.
        assert_eq!(rc4(b"Key", b"Plaintext"), [0xBB, 0xF3, 0x16, 0xE8, 0xD9, 0x40, 0xAF, 0x0A, 0xD3]);
    }

    #[test]
    fn aes_round_trips_with_padding() {
        let key = [7u8; 16];
        for len in [0, 5, 16, 33] {
            let data: Vec<u8> = (0..len as u8).collect();
            let e = aes_cbc_encrypt(&key, &[1; 16], &data);
            assert_eq!(e.len() % 16, 0);
            assert_eq!(aes_cbc_decrypt(&key, &e).unwrap(), data);
        }
        let key = [9u8; 32];
        let e = aes_cbc_encrypt(&key, &[2; 16], b"hello world");
        assert_eq!(aes_cbc_decrypt(&key, &e).unwrap(), b"hello world");
    }
}
