// Copyright 2026 mozrs contributors. Portions derived from Mozc; see LICENSE.
//! Read and build the standalone OSS `dictionary.data` format from pinned Mozc.
//! Provides dictionary lookup and basic minimum-cost sentence conversion.
//! Does not implement full Mozc segmentation, rewriters, prediction or IME APIs.
//! Parsing checks structure eagerly; [`Dictionary::validate`] also decodes all
//! keys and tokens. Lookup returns errors for malformed data it encounters.
pub mod builder;
mod codec;
pub mod connection;
pub mod converter;
pub mod input;
mod reader;
mod trie;

use reader::Reader;
use serde::{Deserialize, Serialize};
use std::fmt;
use trie::Trie;

/// Upstream version whose OSS dictionary format is supported.
pub const UPSTREAM_REVISION: &str = "c4b2aef7c65e61be394bacadaec200eb5b97f91c";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(&'static str);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
fn ensure(condition: bool, message: &'static str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error(message))
    }
}

/// A dictionary token, in upstream storage order (not IME ranking order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub key: String,
    pub value: String,
    pub cost: u16,
    pub lid: u16,
    pub rid: u16,
    /// 0 = ordinary entry; 1 = spelling correction in the system dictionary.
    pub attributes: u8,
}

/// A borrowed dictionary image with validated, expanded traversal indices.
pub struct Dictionary<'a> {
    keys: Trie<'a>,
    values: Trie<'a>,
    rows: Vec<&'a [u8]>,
    pos: Vec<(u16, u16)>,
}

impl<'a> Dictionary<'a> {
    /// Parse little-endian OSS `dictionary.data` (not the outer `mozc.data`).
    /// Structure is checked here; token streams are checked when decoded.
    pub fn from_bytes(data: &'a [u8]) -> Result<Self> {
        let [k, v, t, p] = reader::sections(data)?;
        let keys = Trie::parse(k)?;
        let values = Trie::parse(v)?;
        let rows = trie::token_rows(t, keys.len())?;
        ensure(
            p.len() % 4 == 0 && p.len() <= 256 * 4,
            "invalid frequent POS table",
        )?;
        let mut r = Reader::new(p);
        let mut pos = Vec::new();
        while !r.rest.is_empty() {
            let p = r.u32()?;
            let pair = ((p >> 16) as u16, p as u16);
            ensure(pair.0 <= 0xfff && pair.1 <= 0xfff, "POS ID exceeds 12 bits")?;
            pos.push(pair);
        }
        Ok(Self {
            keys,
            values,
            rows,
            pos,
        })
    }

    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    /// Validate every stored key, value and token; return the token count.
    pub fn validate(&self) -> Result<usize> {
        self.validate_pos_limit(4096)
    }

    pub(crate) fn validate_pos_limit(&self, pos_size: usize) -> Result<usize> {
        for id in 0..self.values.len() {
            codec::decode_value(&self.values.restore(id)?)?;
        }
        let mut count = 0usize;
        for id in 0..self.keys.len() {
            let bytes = self.keys.restore(id)?;
            let encoded = std::str::from_utf8(&bytes).map_err(|_| Error("invalid key UTF-8"))?;
            let key = codec::transform_key(encoded);
            let tokens = self.tokens(id, &key)?;
            ensure(
                tokens
                    .iter()
                    .all(|t| usize::from(t.lid) < pos_size && usize::from(t.rid) < pos_size),
                "dictionary POS ID outside connection matrix",
            )?;
            count = count
                .checked_add(tokens.len())
                .ok_or(Error("token count overflow"))?;
        }
        Ok(count)
    }

    /// Return tokens whose reading equals `query`. No normalization is applied.
    pub fn lookup_exact(&self, query: &str) -> Result<Vec<Token>> {
        let key = codec::transform_key(query);
        let mut node = 0;
        for byte in key.bytes() {
            let Some(next) = self.keys.child(node, byte) else {
                return Ok(Vec::new());
            };
            node = next;
        }
        match self.keys.key_id(node) {
            Some(id) => self.tokens(id, query),
            None => Ok(Vec::new()),
        }
    }

    /// Return tokens whose readings are prefixes of `query` (shortest first).
    /// This is common-prefix lookup, not predictive completion. Kana modifiers
    /// (e.g. か/が, つ/っ) are distinguished, as in upstream's default callback.
    pub fn lookup_prefix(&self, query: &str) -> Result<Vec<Token>> {
        let encoded = codec::transform_key(query);
        let mut node = 0;
        let mut out = Vec::new();
        for (i, byte) in encoded.bytes().enumerate() {
            let Some(next) = self.keys.child(node, byte) else {
                break;
            };
            node = next;
            if let Some(id) = self.keys.key_id(node) {
                let prefix = encoded
                    .get(..i + 1)
                    .ok_or(Error("terminal in the middle of UTF-8"))?;
                out.extend(self.tokens(id, &codec::transform_key(prefix))?);
            }
        }
        Ok(out)
    }

    fn tokens(&self, id: usize, key: &str) -> Result<Vec<Token>> {
        let mut r = Reader::new(self.rows[id]);
        let mut out: Vec<Token> = Vec::new();
        loop {
            let raw_flags = r.byte()?;
            let flags = if raw_flags & 0x40 != 0 {
                raw_flags & 0xc0
            } else {
                raw_flags
            };
            ensure(flags & 0x20 == 0, "reserved token flag")?;
            let previous = out.last();
            let (lid, rid) = match flags & 0x0c {
                0 => *self
                    .pos
                    .get(r.byte()? as usize)
                    .ok_or(Error("frequent POS index out of bounds"))?,
                4 => {
                    let a = u16::from(r.byte()?);
                    let b = u16::from(r.byte()?);
                    let c = u16::from(r.byte()?);
                    (a | ((b & 15) << 8), (b >> 4) | (c << 4))
                }
                8 => {
                    let p = u16::from_le_bytes(r.take(2)?.try_into().unwrap());
                    (p, p)
                }
                _ => {
                    let p = previous.ok_or(Error("first token reuses POS"))?;
                    (p.lid, p.rid)
                }
            };
            ensure(lid <= 0xfff && rid <= 0xfff, "POS ID exceeds 12 bits")?;
            let high = r.byte()?;
            let cost = if high & 0x80 != 0 {
                u16::from(high & 0x7f) << 8
            } else {
                (u16::from(high) << 8) | u16::from(r.byte()?)
            };
            let value = match flags & 3 {
                0 => {
                    let low = usize::from(r.byte()?) | (usize::from(r.byte()?) << 8);
                    let high = if flags & 0x40 != 0 {
                        raw_flags & 0x3f
                    } else {
                        r.byte()?
                    };
                    ensure(high <= 0x3f, "value ID exceeds 22 bits")?;
                    codec::decode_value(&self.values.restore(low | (usize::from(high) << 16))?)?
                }
                1 => key.to_owned(),
                2 => codec::katakana(key),
                _ => previous
                    .ok_or(Error("first token reuses value"))?
                    .value
                    .clone(),
            };
            out.push(Token {
                key: key.into(),
                value,
                cost,
                lid,
                rid,
                attributes: u8::from(flags & 0x10 != 0),
            });
            if flags & 0x80 != 0 {
                r.zeros()?;
                return Ok(out);
            }
        }
    }
}
