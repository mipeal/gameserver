//! The Rust port of the gameserver-used subset of `gamelib/` (ADR-013 in rust/DECISIONS.md):
//! flag generation + regex, flag-id generation (all types incl. username wordlist),
//! and the flag_id_type validators. Service-facing helpers (ServiceInterface, assert_*)
//! and the OS-random generators stay in Python on purpose.
//!
//! The RNG reproduces CPython's `random.Random` bit-for-bit (Mersenne Twister with
//! `sha512(seed)+seed` big-endian seeding and `init_by_array`), because flag IDs are
//! derived from an HMAC seed and must be identical across the cutover.

#![forbid(unsafe_code)]

use base64::Engine;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use std::sync::LazyLock;

/// `config.SECRET_FLAG_KEY` fallback from `gamelib/gamelib.py` when saarctf_commons is absent.
const DEFAULT_SECRET_FLAG_KEY: &[u8] = b"\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\
                                          \x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
/// `config.FLAG_PREFIX` fallback.
const DEFAULT_FLAG_PREFIX: &str = "SAAR";
const MAC_LENGTH: usize = 16;

static ADJECTIVES: LazyLock<Vec<String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/USERNAME_ADJECTIVES.json"))
        .expect("INFALLIBLE: generated data file")
});
static NOUNS: LazyLock<Vec<String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/USERNAME_NOUNS.json"))
        .expect("INFALLIBLE: generated data file")
});

// ---------------------------------------------------------------------------
// CPython-compatible random.Random (Mersenne Twister)
// ---------------------------------------------------------------------------

struct PyRandom {
    mt: [u32; 624],
    index: usize,
}

impl PyRandom {
    /// `random.Random(bytes_seed)`: `int.from_bytes(seed + sha512(seed), 'big')`, then
    /// seed the MT with the 32-bit little-endian words of that integer (CPython
    /// `random_seed` -> `init_by_array`).
    fn from_bytes_seed(seed: &[u8]) -> Self {
        let mut hasher = Sha512::new();
        hasher.update(seed);
        let digest = hasher.finalize();
        let bytes = [seed, &digest].concat();
        // Big-endian integer: the LAST 4 bytes are the least-significant word.
        let mut key = Vec::with_capacity(bytes.len() / 4 + 1);
        let mut end = bytes.len();
        while end > 0 {
            let start = end.saturating_sub(4);
            let mut word = [0u8; 4];
            word[4 - (end - start)..].copy_from_slice(&bytes[start..end]);
            key.push(u32::from_be_bytes(word));
            end = start;
        }
        if key.is_empty() {
            key.push(0);
        }
        Self::init_by_array(&key)
    }

    fn init_by_array(key: &[u32]) -> Self {
        let mut mt = [0u32; 624];
        // init_genrand(19650218)
        mt[0] = 19650218;
        for i in 1..624 {
            mt[i] = 1812433253u32
                .wrapping_mul(mt[i - 1] ^ (mt[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        let mut i = 1usize;
        let mut j = 0usize;
        let mut k = 624.max(key.len());
        while k > 0 {
            mt[i] = (mt[i] ^ (mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_mul(1664525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= 624 {
                mt[0] = mt[623];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
            k -= 1;
        }
        k = 623;
        while k > 0 {
            mt[i] = (mt[i] ^ (mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_mul(1566083941))
                .wrapping_sub(i as u32);
            i += 1;
            if i >= 624 {
                mt[0] = mt[623];
                i = 1;
            }
            k -= 1;
        }
        mt[0] = 0x8000_0000;
        PyRandom { mt, index: 624 } // force a twist on first use
    }

    fn genrand_u32(&mut self) -> u32 {
        if self.index >= 624 {
            self.twist();
        }
        let mut y = self.mt[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^= y >> 18;
        y
    }

    fn twist(&mut self) {
        const M: usize = 397;
        const MATRIX_A: u32 = 0x9908_b0df;
        for i in 0..624 {
            let y = (self.mt[i] & 0x8000_0000) | (self.mt[(i + 1) % 624] & 0x7fff_ffff);
            let mut next = self.mt[(i + M) % 624] ^ (y >> 1);
            if y & 1 != 0 {
                next ^= MATRIX_A;
            }
            self.mt[i] = next;
        }
        self.index = 0;
    }

    /// CPython `random_getrandbits`: little-endian words, last word shifted right.
    fn getrandbits(&mut self, k: u32) -> u64 {
        debug_assert!(k > 0);
        let words = (k as usize - 1) / 32 + 1;
        let mut result = 0u128;
        let mut k = k;
        for i in 0..words {
            let mut r = self.genrand_u32();
            if k < 32 {
                r >>= 32 - k;
            }
            result |= (r as u128) << (32 * i);
            k = k.saturating_sub(32);
        }
        result as u64
    }

    /// CPython `random()`: 53-bit resolution from two draws.
    fn random(&mut self) -> f64 {
        let a = (self.genrand_u32() >> 5) as f64;
        let b = (self.genrand_u32() >> 6) as f64;
        (a * 67_108_864.0 + b) * (1.0 / 9_007_199_254_740_992.0)
    }

    /// `_randbelow_with_getrandbits(n)`.
    fn randbelow(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        let k = 64 - n.leading_zeros();
        let mut r = self.getrandbits(k);
        while r >= n {
            r = self.getrandbits(k);
        }
        r
    }

    /// `random.randint(a, b)` = `a + _randbelow(b - a + 1)`.
    fn randint(&mut self, a: i64, b: i64) -> i64 {
        a + self.randbelow((b - a + 1) as u64) as i64
    }

    /// `random.choice(seq)`.
    fn choice<'a, T>(&mut self, seq: &'a [T]) -> &'a T {
        &seq[self.randbelow(seq.len() as u64) as usize]
    }

    /// `random.choices(population, k=k)`: `floor(random() * len)` per item.
    fn choices(&mut self, population: &str, k: i64) -> String {
        let chars: Vec<char> = population.chars().collect();
        (0..k.max(0))
            .map(|_| {
                let idx = (self.random() * chars.len() as f64).floor() as usize;
                chars[idx.min(chars.len() - 1)]
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// flag generation (gamelib/gamelib.py)
// ---------------------------------------------------------------------------

/// Port of `get_flag` (`gamelib/gamelib.py:54`) with the fallback secret key.
pub fn get_flag(team_id: i64, service_id: i64, tick: i64, payload: i64) -> Result<String, String> {
    get_flag_with_key(
        DEFAULT_SECRET_FLAG_KEY.to_vec(),
        team_id,
        service_id,
        tick,
        payload,
    )
}

/// `get_flag` with an explicit `SECRET_FLAG_KEY` (the real config value).
pub fn get_flag_with_key(
    key: Vec<u8>,
    team_id: i64,
    service_id: i64,
    tick: i64,
    payload: i64,
) -> Result<String, String> {
    let data = pack_hhhh(tick & 0xffff, team_id, service_id & 0xffff, payload)?;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(&key).expect("INFALLIBLE: HMAC accepts any key length");
    mac.update(&data);
    let digest = mac.finalize().into_bytes();
    let mut bytes = data;
    bytes.extend_from_slice(&digest[..MAC_LENGTH]);
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!(
        "{}{{{}}}",
        DEFAULT_FLAG_PREFIX,
        encoded.replace('+', "-").replace('/', "_")
    ))
}

/// `struct.pack('<HHHH', ...)` — every field must fit `H` like the original.
fn pack_hhhh(a: i64, b: i64, c: i64, d: i64) -> Result<Vec<u8>, String> {
    let pack = |v: i64| -> Result<[u8; 2], String> {
        // Kind prefix "struct.error:" makes the binding raise Python's real struct.error
        // (type name `error`), like `struct.pack('<H', ...)` does.
        u16::try_from(v)
            .map(|x| x.to_le_bytes())
            .map_err(|_| "struct.error: 'H' format requires 0 <= number <= 65535".to_string())
    };
    let mut out = Vec::with_capacity(8);
    for v in [pack(a)?, pack(b)?, pack(c)?, pack(d)?] {
        out.extend_from_slice(&v);
    }
    Ok(out)
}

/// Port of `get_flag_regex().pattern` (`gamelib/gamelib.py:50`) — the regex source string.
pub fn get_flag_regex_pattern() -> String {
    format!("{}{{[A-Za-z0-9-_]{{32}}}}", DEFAULT_FLAG_PREFIX)
}

// ---------------------------------------------------------------------------
// flag ids (gamelib/flag_ids.py)
// ---------------------------------------------------------------------------

/// Port of `generate_flag_id` (`gamelib/flag_ids.py:16`) with the fallback secret key.
pub fn generate_flag_id(
    flag_id_type: String,
    service_id: i64,
    team_id: i64,
    tick: i64,
    index: i64,
) -> Result<String, String> {
    generate_flag_id_with_key(
        DEFAULT_SECRET_FLAG_KEY.to_vec(),
        flag_id_type,
        service_id,
        team_id,
        tick,
        index,
    )
}

/// `generate_flag_id` with an explicit `SECRET_FLAG_KEY` (the real config value).
pub fn generate_flag_id_with_key(
    key: Vec<u8>,
    flag_id_type: String,
    service_id: i64,
    team_id: i64,
    tick: i64,
    index: i64,
) -> Result<String, String> {
    let mut mac =
        Hmac::<Sha1>::new_from_slice(&key).expect("INFALLIBLE: HMAC accepts any key length");
    mac.update(format!("{service_id}|{team_id}|{tick}|{index}").as_bytes());
    let seed = mac.finalize().into_bytes();
    generate_flag_id_rnd(flag_id_type, seed.to_vec())
}

/// `generate_flag_id_rnd` (`gamelib/flag_ids.py:39`): dispatch on the flag-id type,
/// driven by a CPython-compatible RNG seeded with `seed`.
pub fn generate_flag_id_rnd(flag_id_type: String, seed: Vec<u8>) -> Result<String, String> {
    let mut rnd = PyRandom::from_bytes_seed(&seed);
    flag_id_dispatch(&flag_id_type, &mut rnd)
}

fn flag_id_dispatch(flag_id_type: &str, rnd: &mut PyRandom) -> Result<String, String> {
    if flag_id_type.starts_with("username") {
        let camelcase = !flag_id_type.contains(":lower");
        Ok(generate_username(rnd, camelcase))
    } else if let Some(rest) = flag_id_type.strip_prefix("hex") {
        let len = parse_py_int(rest)?;
        Ok(rnd.choices("0123456789abcde", len))
    } else if let Some(rest) = flag_id_type.strip_prefix("alphanum") {
        let len = parse_py_int(rest)?;
        let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
            .chars()
            .collect();
        Ok((0..len.max(0))
            .map(|_| rnd.choice(&chars).to_string())
            .collect())
    } else if flag_id_type == "email" {
        Ok(format!(
            "{}@{}.com",
            generate_username(rnd, false),
            generate_username(rnd, false)
        ))
    } else if let Some(rest) = flag_id_type.strip_prefix("choose_k_from:") {
        let mut parts = rest.splitn(3, ':');
        let _ = parts.next();
        let k = parse_py_int(parts.next().unwrap_or(""))?;
        let alpha = parts.next().unwrap_or("");
        Ok(rnd.choices(alpha, k))
    } else if let Some(pattern) = flag_id_type.strip_prefix("pattern:") {
        // re.sub(r"\$\{(.*?)}", ...) — non-greedy up to the first `}`.
        let mut out = String::with_capacity(pattern.len());
        let mut rest = pattern;
        while let Some(dollar) = rest.find("${") {
            out.push_str(&rest[..dollar]);
            let after = &rest[dollar + 2..];
            match after.find('}') {
                Some(close) => {
                    out.push_str(&flag_id_dispatch(&after[..close], rnd)?);
                    rest = &after[close + 1..];
                }
                None => {
                    // No closing brace: the regex would not match; emit the rest verbatim.
                    out.push_str("${");
                    out.push_str(after);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        Ok(out)
    } else {
        Err(format!(
            "ValueError: Invalid FlagId type requested: {}",
            py_repr_str(flag_id_type)
        ))
    }
}

/// Python `repr()` of a `str`: single quotes, or double quotes when it contains a `'`.
fn py_repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if c == quote => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Python `int()`: strict decimal (leading/trailing spaces allowed), else the
/// exact ValueError message.
fn parse_py_int(s: &str) -> Result<i64, String> {
    s.trim().parse::<i64>().map_err(|_| {
        format!(
            "ValueError: invalid literal for int() with base 10: {}",
            py_repr_str(s)
        )
    })
}

/// Port of `usernames.generate_username` (`gamelib/usernames.py:5709`).
fn generate_username(rnd: &mut PyRandom, camelcase: bool) -> String {
    let words = if rnd.randint(0, 100) < 90 { 3 } else { 2 };
    let name = generate_name(rnd, words, camelcase, true, "");
    format!("{}{}", name, rnd.randint(1, 9999))
}

/// Port of `usernames.generate_name` (`gamelib/usernames.py:5695`) with an explicit RNG.
fn generate_name(
    rnd: &mut PyRandom,
    words: i64,
    camelcase: bool,
    alphanum: bool,
    sep: &str,
) -> String {
    let mut components: Vec<String> = Vec::new();
    for _ in 0..(words - 1).max(0) {
        components.push(rnd.choice(ADJECTIVES.as_slice()).clone());
    }
    components.push(rnd.choice(NOUNS.as_slice()).clone());
    if camelcase {
        components = components.into_iter().map(|c| ucfirst_lossy(&c)).collect();
    }
    let mut name = components.join(sep);
    if alphanum {
        name = name.replace('-', "");
    }
    name
}

fn ucfirst_lossy(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Port of `usernames.ucfirst` (`gamelib/usernames.py:5691`).
pub fn ucfirst(s: String) -> Result<String, String> {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => Ok(first.to_uppercase().collect::<String>() + chars.as_str()),
        None => Err("IndexError: string index out of range".to_string()),
    }
}

// ---------------------------------------------------------------------------
// flag_id_type validators (gamelib/flag_ids.py:95)
// ---------------------------------------------------------------------------

/// Port of `is_generatable_flag_id`.
pub fn is_generatable_flag_id(flag_id_type: String) -> Result<bool, String> {
    match generate_flag_id(flag_id_type, 1, 1, 1, 0) {
        Ok(_) => Ok(true),
        Err(e) if e.starts_with("ValueError:") => Ok(false),
        Err(e) => Err(e),
    }
}

/// Port of `is_valid_flag_id`.
pub fn is_valid_flag_id(flag_id_type: String) -> Result<bool, String> {
    if flag_id_type == "custom" {
        Ok(true)
    } else {
        is_generatable_flag_id(flag_id_type)
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_format_matches_recorded_sample() {
        // Recorded from the original: get_flag(1, 1, 1, 0) with the zero key.
        assert_eq!(
            get_flag(1, 1, 1, 0).unwrap(),
            "SAAR{AQABAAEAAAA1gl4THL8cvFriWd_mag-H}"
        );
    }

    #[test]
    fn flag_id_hex_matches_recorded_sample() {
        // Recorded from the original: generate_flag_id('hex10', 1, 2, 3, 0).
        assert_eq!(
            generate_flag_id("hex10".into(), 1, 2, 3, 0).unwrap(),
            "b9bdbab4aa"
        );
    }

    #[test]
    fn mt_seeding_matches_cpython() {
        // Recorded from the original: random.Random(b'\x01\x02\x03') ->
        // generate_username(generator=rnd) == "BetterDeliriousSatire1533".
        let mut rnd = PyRandom::from_bytes_seed(b"\x01\x02\x03");
        assert_eq!(
            generate_username(&mut rnd, true),
            "BetterDeliriousSatire1533"
        );
    }

    #[test]
    fn invalid_type_error_message() {
        let err = generate_flag_id("foobar".into(), 1, 1, 1, 0).unwrap_err();
        assert_eq!(err, "ValueError: Invalid FlagId type requested: 'foobar'");
        assert!(!is_generatable_flag_id("foobar".into()).unwrap());
        assert!(is_generatable_flag_id("hex10".into()).unwrap());
        assert!(is_valid_flag_id("custom".into()).unwrap());
    }

    #[test]
    fn pattern_dispatch() {
        // Same seed, so pattern parts must equal the direct generations.
        let seed = b"abc";
        let pattern =
            generate_flag_id_rnd("pattern:${hex4}_${username:lower}".into(), seed.to_vec())
                .unwrap();
        let mut rnd = PyRandom::from_bytes_seed(seed);
        let hex = flag_id_dispatch("hex4", &mut rnd).unwrap();
        let user = flag_id_dispatch("username:lower", &mut rnd).unwrap();
        assert_eq!(pattern, format!("{hex}_{user}"));
    }

    #[test]
    fn struct_range_errors() {
        assert!(get_flag(70_000, 1, 1, 0).is_err());
        assert!(get_flag(-1, 1, 1, 0).is_err());
        assert!(get_flag(1, 1, 1, -1).is_err());
        assert!(get_flag(65_535, 65_535, 65_535, 65_535).is_ok());
    }

    #[test]
    fn ucfirst_edge_cases() {
        assert_eq!(ucfirst("hello".into()).unwrap(), "Hello");
        assert!(ucfirst(String::new()).is_err());
    }
}
