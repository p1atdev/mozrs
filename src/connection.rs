// Copyright 2026 mozrs contributors. Portions derived from Mozc; see LICENSE.
//! Pinned OSS connection.data, resolution 1 (lossless unsigned 16-bit costs).
//! The reader expands the compressed matrix for constant-time checked access.
use crate::{Error, Result, ensure, reader::Reader};
use std::{collections::HashMap, io::BufRead};

pub const INVALID_COST: u16 = 30000;

pub struct Connection {
    size: usize,
    costs: Vec<u16>,
}

impl Connection {
    /// Row-major matrix: row = preceding right POS, column = following left POS.
    /// Cost 30000 is a finite penalty, not an impossible edge.
    pub fn from_costs(size: usize, costs: Vec<u16>) -> Result<Self> {
        ensure(
            (1..=4096).contains(&size),
            "connection size must be 1..4096",
        )?;
        ensure(
            costs.len() == size * size,
            "connection matrix size mismatch",
        )?;
        ensure(costs[0] == 0, "BOS-to-EOS cost must be zero")?;
        Ok(Self { size, costs })
    }
    pub fn size(&self) -> usize {
        self.size
    }
    pub fn cost(&self, rid: u16, lid: u16) -> Result<u16> {
        ensure(
            usize::from(rid) < self.size && usize::from(lid) < self.size,
            "connection POS ID out of range",
        )?;
        Ok(self.costs[usize::from(rid) * self.size + usize::from(lid)])
    }
    pub fn costs(&self) -> &[u16] {
        &self.costs
    }

    /// Read the complete pinned single-column text matrix; add special POS rows
    /// and columns with cost 30000, except transitions from/to BOS/EOS (0).
    /// Missing/extra cells are errors instead of implicit zero-filled cells.
    pub fn from_text(reader: impl BufRead, id_def: &str, special_pos: &str) -> Result<Self> {
        let ids = pos_definitions(id_def)?;
        let base = ids.len();
        let extra = definition_lines(special_pos).count();
        let size = base + extra;
        ensure(size <= 4096, "connection size exceeds 4096")?;
        let mut lines = reader.lines();
        let mut next = || -> Result<Option<String>> {
            for line in lines.by_ref() {
                let line = line.map_err(|_| Error("cannot read connection text as UTF-8"))?;
                let line = line.trim();
                if !line.is_empty() && !line.starts_with('#') {
                    return Ok(Some(line.to_owned()));
                }
            }
            Ok(None)
        };
        let declared = next()?
            .ok_or(Error("missing connection matrix size"))?
            .parse::<usize>()
            .map_err(|_| Error("invalid connection matrix size"))?;
        ensure(declared == base, "connection size does not match id.def")?;
        let mut costs = vec![INVALID_COST; size * size];
        for rid in 0..base {
            for lid in 0..base {
                costs[rid * size + lid] = next()?
                    .ok_or(Error("missing connection cost"))?
                    .parse()
                    .map_err(|_| Error("invalid connection cost"))?;
            }
        }
        ensure(next()?.is_none(), "extra connection costs")?;
        costs[0] = 0;
        for special in base..size {
            costs[special] = 0;
            costs[special * size] = 0;
        }
        Self::from_costs(size, costs)
    }

    /// Produce the same mode-compressed, two-level bitmap format as the pinned
    /// upstream Python generator with --use_1byte_cost=false.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut defaults = Vec::new();
        for row in self.costs.chunks_exact(self.size) {
            let mut counts = HashMap::new();
            for &cost in row {
                if cost != INVALID_COST {
                    *counts.entry(cost).or_insert(0usize) += 1;
                }
            }
            // Python dict insertion order breaks mode ties by first appearance.
            let mut best = None;
            let mut frequency = 0;
            for &cost in row {
                let count = counts.get(&cost).copied().unwrap_or(0);
                if count > frequency {
                    best = Some(cost);
                    frequency = count;
                }
            }
            defaults.push(best.ok_or(Error("connection row has no non-sentinel cost"))?);
        }
        let mut image = Vec::new();
        for n in [0xcdab, 1, self.size as u16, self.size as u16] {
            image.extend(n.to_le_bytes());
        }
        for &cost in &defaults {
            image.extend(cost.to_le_bytes());
        }
        pad4(&mut image);
        for (row, &default) in self.costs.chunks_exact(self.size).zip(&defaults) {
            let mut chunks = Vec::new();
            let mut compact = Vec::new();
            let mut values = Vec::new();
            for chunk in row.chunks(8) {
                let nonempty = chunk.iter().any(|&c| c != default);
                chunks.push(nonempty);
                if nonempty {
                    for &cost in chunk {
                        compact.push(cost != default);
                        if cost != default {
                            values.extend(cost.to_le_bytes());
                        }
                    }
                }
            }
            let chunks = encode_bits(&chunks);
            let compact = encode_bits(&compact);
            pad4(&mut values);
            for n in [compact.len(), values.len()] {
                image.extend(
                    u16::try_from(n)
                        .map_err(|_| Error("connection row too large"))?
                        .to_le_bytes(),
                );
            }
            image.extend(chunks);
            image.extend(compact);
            image.extend(values);
        }
        Ok(image)
    }

    pub fn from_bytes(image: &[u8]) -> Result<Self> {
        let mut r = Reader::new(image);
        ensure(u16_field(&mut r)? == 0xcdab, "invalid connection magic")?;
        ensure(
            u16_field(&mut r)? == 1,
            "only lossless resolution-1 connection data is supported",
        )?;
        let size = usize::from(u16_field(&mut r)?);
        ensure(
            size == usize::from(u16_field(&mut r)?),
            "connection matrix is not square",
        )?;
        ensure(
            (1..=4096).contains(&size),
            "connection size must be 1..4096",
        )?;
        let mut defaults = Vec::with_capacity(size);
        for _ in 0..size {
            defaults.push(u16_field(&mut r)?);
        }
        ensure(
            r.take((size % 2) * 2)?.iter().all(|&v| v == 0),
            "nonzero connection padding",
        )?;
        let mut costs = Vec::with_capacity(size * size);
        let chunk_count = size.div_ceil(8);
        let chunk_bytes = chunk_count.div_ceil(32) * 4;
        for default in defaults {
            let compact_size = usize::from(u16_field(&mut r)?);
            let values_size = usize::from(u16_field(&mut r)?);
            ensure(
                compact_size % 4 == 0 && values_size % 4 == 0,
                "unaligned connection row",
            )?;
            let mut chunks = crate::reader::Bits::new(r.take(chunk_bytes)?);
            let mut compact = crate::reader::Bits::new(r.take(compact_size)?);
            let mut values = Reader::new(r.take(values_size)?);
            for start in (0..size).step_by(8) {
                if chunks.next()? {
                    for _ in start..(start + 8).min(size) {
                        costs.push(if compact.next()? {
                            u16_field(&mut values)?
                        } else {
                            default
                        });
                    }
                } else {
                    costs.resize(costs.len() + (size - start).min(8), default);
                }
            }
            chunks.finish()?;
            compact.finish()?;
            ensure(values.rest.len() <= 2, "extra connection values")?;
            values.zeros()?;
        }
        ensure(r.rest.is_empty(), "trailing connection data")?;
        Self::from_costs(size, costs)
    }
}

pub fn unknown_pos_id(id_def: &str) -> Result<u16> {
    pos_definitions(id_def)?
        .iter()
        .position(|s| s.starts_with("名詞,サ変接続"))
        .map(|id| id as u16)
        .ok_or(Error("Unknown POS feature missing in id.def"))
}

fn pos_definitions(id_def: &str) -> Result<Vec<&str>> {
    let mut ids = Vec::new();
    for line in definition_lines(id_def) {
        let mut fields = line.split_whitespace();
        let id: usize = fields
            .next()
            .ok_or(Error("missing POS ID"))?
            .parse()
            .map_err(|_| Error("invalid POS ID"))?;
        let feature = fields.next().ok_or(Error("missing POS feature"))?;
        ensure(
            id == ids.len() && id < 4096 && fields.next().is_none(),
            "POS IDs must be consecutive from zero",
        )?;
        ids.push(feature);
    }
    ensure(!ids.is_empty(), "empty POS definitions")?;
    Ok(ids)
}
fn definition_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
}
fn u16_field(r: &mut Reader<'_>) -> Result<u16> {
    Ok(u16::from_le_bytes(r.take(2)?.try_into().unwrap()))
}
fn pad4(out: &mut Vec<u8>) {
    out.resize(out.len().next_multiple_of(4), 0);
}
fn encode_bits(bits: &[bool]) -> Vec<u8> {
    let mut out = vec![0; bits.len().div_ceil(32) * 4];
    for (i, &bit) in bits.iter().enumerate() {
        if bit {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    out
}
