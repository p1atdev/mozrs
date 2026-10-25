// Copyright 2026 mozrs contributors. Portions derived from Mozc; see LICENSE.
use crate::{Error, Result, ensure};

pub(crate) struct Reader<'a> {
    pub rest: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(rest: &'a [u8]) -> Self {
        Self { rest }
    }
    pub fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let out = self.rest.get(..len).ok_or(Error("truncated data"))?;
        self.rest = &self.rest[len..];
        Ok(out)
    }
    pub fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn size(&mut self) -> Result<usize> {
        Ok(self.u32()? as usize)
    }
    pub fn zeros(&self) -> Result<()> {
        ensure(self.rest.iter().all(|&x| x == 0), "nonzero padding")
    }
}

pub(crate) struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn next(&mut self) -> Result<bool> {
        let byte = self
            .data
            .get(self.pos / 8)
            .ok_or(Error("truncated bit vector"))?;
        let bit = byte & (1 << (self.pos % 8)) != 0;
        self.pos += 1;
        Ok(bit)
    }
    pub fn finish(mut self) -> Result<()> {
        while self.pos / 8 < self.data.len() {
            ensure(!self.next()?, "nonzero bit-vector padding")?;
        }
        Ok(())
    }
}

// Mozc's section names are the one-byte strings k, v, t, p. This is the
// one-byte specialization of base/hash.cc's LegacyFingerprintWithSeed.
fn fingerprint32(name: u8, seed: u32) -> u32 {
    let mut a = 0x9e3779b9u32.wrapping_add(u32::from(name));
    let mut b = 0x9e3779b9u32;
    let mut c = seed.wrapping_add(1);
    for (right_a, left_b, right_c) in [(13, 8, 13), (12, 16, 5), (3, 10, 15)] {
        a = a.wrapping_sub(b).wrapping_sub(c) ^ (c >> right_a);
        b = b.wrapping_sub(c).wrapping_sub(a) ^ (a << left_b);
        c = c.wrapping_sub(a).wrapping_sub(b) ^ (b >> right_c);
    }
    c
}

pub(crate) fn fingerprint(name: u8, seed: u32) -> [u8; 8] {
    let hi = fingerprint32(name, seed);
    let lo = fingerprint32(name, 0x7a63);
    let mut result = (u64::from(hi) << 32) | u64::from(lo);
    if hi == 0 && lo < 2 {
        result ^= 0x130f9bef94a0a928;
    }
    result.to_le_bytes()
}

pub(crate) fn sections(data: &[u8]) -> Result<[&[u8]; 4]> {
    let mut r = Reader::new(data);
    ensure(
        r.u32()? == 20110701,
        "expected a Mozc OSS dictionary.data file",
    )?;
    let seed = r.u32()?;
    let names = b"kvtp".map(|b| fingerprint(b, seed));
    let mut sections = [None; 4];
    loop {
        let len = r.size()?;
        if len == 0 {
            break;
        }
        ensure(len <= i32::MAX as usize, "invalid section length")?;
        let name = r.take(8)?;
        let slot = names
            .iter()
            .position(|n| n == name)
            .ok_or(Error("unknown section"))?;
        ensure(sections[slot].is_none(), "duplicate section")?;
        sections[slot] = Some(r.take(len)?);
        ensure(
            r.take((4 - len % 4) % 4)?.iter().all(|&b| b == 0),
            "invalid section padding",
        )?;
    }
    ensure(r.rest.is_empty(), "trailing data after dictionary")?;
    let [k, v, t, p] = sections;
    Ok([
        k.ok_or(Error("missing key section"))?,
        v.ok_or(Error("missing value section"))?,
        t.ok_or(Error("missing token section"))?,
        p.ok_or(Error("missing POS section"))?,
    ])
}
