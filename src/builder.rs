// Copyright 2026 mozrs contributors. Portions derived from Mozc; see LICENSE.
//! Build the pinned OSS dictionary format without Mozc C++ or FFI.
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::{Error, Result, Token, codec, ensure, reader::fingerprint};

/// Upstream's small-cost encoding discards the low eight bits of eligible costs.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    /// Minimum reading length in Unicode scalars. Upstream's default is 6.
    /// Set to `usize::MAX` to disable this lossy optimization.
    pub min_key_length_for_small_cost: usize,
}
impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            min_key_length_for_small_cost: 6,
        }
    }
}

struct Entry {
    token: Token,
    value_id: i32,
    value_type: u8,
}

/// Encode prepared tokens. Does not normalize text or insert reading corrections;
/// use [`crate::input`] for Mozc's TSV loading semantics. Input order breaks ties.
/// Duplicate tokens are retained. POS IDs must fit 12 bits, costs 15 bits, and
/// attributes must be 0 or 1. Encoded keys/values are limited to 256 bytes to stay
/// within the upstream reader's fixed restore buffer. NULs are rejected.
pub fn build(tokens: Vec<Token>, options: &BuildOptions) -> Result<Vec<u8>> {
    let mut entries = Vec::with_capacity(tokens.len());
    let mut value_words = HashSet::new();
    let mut frequencies = BTreeMap::new();
    for token in tokens {
        ensure(
            !token.key.is_empty() && !token.value.is_empty(),
            "empty key or value",
        )?;
        ensure(
            !token.key.contains('\0') && !token.value.contains('\0'),
            "NUL in key or value",
        )?;
        ensure(
            token.lid <= 4095 && token.rid <= 4095,
            "POS ID exceeds 12 bits",
        )?;
        ensure(token.cost <= 32767, "cost exceeds 15 bits")?;
        ensure(token.attributes <= 1, "unsupported token attributes")?;
        ensure(
            codec::transform_key(&token.key).len() <= 256,
            "encoded key exceeds 256 bytes",
        )?;
        let value = codec::encode_value(&token.value);
        ensure(value.len() <= 256, "encoded value exceeds 256 bytes")?;
        let value_type = if token.key == token.value {
            1
        } else if codec::katakana(&token.key) == token.value {
            2
        } else {
            0
        };
        if value_type == 0 {
            value_words.insert(value);
        }
        *frequencies.entry((token.lid, token.rid)).or_insert(0usize) += 1;
        entries.push(Entry {
            token,
            value_id: -1,
            value_type,
        });
    }
    let (value_image, value_ids) = trie(value_words.into_iter().collect())?;
    ensure(value_ids.len() <= 0x400000, "value trie exceeds 22-bit IDs")?;
    for entry in &mut entries {
        if let Some(&id) = value_ids.get(&codec::encode_value(&entry.token.value)) {
            entry.value_id = id as i32;
        }
    }
    drop(value_ids);
    let frequent = frequent_pos(frequencies);
    let heterophones = heterophones(&entries);
    // Both sorts must be stable: upstream preserves input order for exact ties.
    entries.sort_by(|a, b| a.token.key.cmp(&b.token.key));
    let mut keys = Vec::new();
    let mut rows = Vec::new();
    for group in entries.chunk_by_mut(|a, b| a.token.key == b.token.key) {
        keys.push(codec::transform_key(&group[0].token.key).into_bytes());
        group.sort_by(|a, b| {
            b.token
                .lid
                .cmp(&a.token.lid)
                .then_with(|| b.token.rid.cmp(&a.token.rid))
                .then_with(|| a.value_id.cmp(&b.value_id))
                .then_with(|| a.token.attributes.cmp(&b.token.attributes))
        });
        let small = group[0].token.key.chars().count() >= options.min_key_length_for_small_cost
            && !group
                .windows(2)
                .any(|w| (w[0].token.lid, w[0].token.rid) == (w[1].token.lid, w[1].token.rid))
            && !group.iter().any(|e| heterophones.contains(&e.token.value));
        rows.push(encode_tokens(group, &frequent, small));
    }
    let (key_image, key_ids) = trie(keys.clone())?;
    let mut ordered_rows = vec![Vec::new(); rows.len()];
    for (key, row) in keys.into_iter().zip(rows) {
        ordered_rows[key_ids[&key]] = row;
    }
    ordered_rows.push(vec![0xff]);
    let token_image = token_array(ordered_rows)?;
    let mut pos_image = vec![0; 256 * 4];
    for ((lid, rid), id) in frequent {
        let pos = (u32::from(lid) << 16) | u32::from(rid);
        let offset = usize::from(id) * 4;
        pos_image[offset..offset + 4].copy_from_slice(&pos.to_le_bytes());
    }
    let seed = 2135654146;
    let mut image = Vec::new();
    image.extend(20110701u32.to_le_bytes());
    image.extend(u32::to_le_bytes(seed));
    for (name, section) in [
        (b'v', value_image),
        (b't', token_image),
        (b'k', key_image),
        (b'p', pos_image),
    ] {
        ensure(
            section.len() <= i32::MAX as usize,
            "dictionary section exceeds 2 GiB",
        )?;
        push_size(&mut image, section.len())?;
        image.extend(fingerprint(name, seed));
        image.extend(section);
        pad4(&mut image);
    }
    image.extend(0u32.to_le_bytes());
    Ok(image)
}

fn frequent_pos(frequencies: BTreeMap<(u16, u16), usize>) -> BTreeMap<(u16, u16), u8> {
    let mut histogram = BTreeMap::new();
    for &frequency in frequencies.values() {
        *histogram.entry(frequency).or_insert(0usize) += 1;
    }
    let mut threshold = usize::MAX;
    let mut count = 0;
    for (frequency, size) in histogram.into_iter().rev() {
        if count + size > 255 {
            break;
        }
        count += size;
        threshold = frequency;
    }
    frequencies
        .into_iter()
        .filter(|(_, f)| *f >= threshold)
        .enumerate()
        .map(|(id, (pos, _))| (pos, id as u8))
        .collect()
}

fn heterophones(entries: &[Entry]) -> HashSet<String> {
    let mut first = HashMap::new();
    let mut result = HashSet::new();
    for entry in entries {
        let t = &entry.token;
        if *first.entry(&t.value).or_insert(&t.key) != &t.key {
            result.insert(t.value.clone());
        }
    }
    result
}

fn encode_tokens(entries: &[Entry], frequent: &BTreeMap<(u16, u16), u8>, small: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let t = &e.token;
        let previous = i.checked_sub(1).map(|j| &entries[j].token);
        let pos = (t.lid, t.rid);
        let pos_type = if previous.is_some_and(|p| (p.lid, p.rid) == pos) {
            12
        } else if frequent.contains_key(&pos) {
            0
        } else if t.lid == t.rid {
            8
        } else {
            4
        };
        let value_type = if e.value_type == 0 && previous.is_some_and(|p| p.value == t.value) {
            3
        } else {
            e.value_type
        };
        let mut flags = pos_type | value_type | (t.attributes << 4);
        let crammed = flags == 0;
        if crammed {
            flags |= 0x40 | (e.value_id >> 16) as u8;
        }
        if i + 1 == entries.len() {
            flags |= 0x80;
        }
        out.push(flags);
        match pos_type {
            0 => out.push(frequent[&pos]),
            4 => out.extend([
                t.lid as u8,
                ((t.rid << 4) | (t.lid >> 8)) as u8,
                (t.rid >> 4) as u8,
            ]),
            8 => out.extend(t.lid.to_le_bytes()),
            _ => (),
        }
        if small && t.cost >= 256 {
            out.push((t.cost >> 8) as u8 | 0x80);
        } else {
            out.extend(t.cost.to_be_bytes());
        }
        if value_type == 0 {
            let bytes = e.value_id.to_le_bytes();
            out.extend(&bytes[..if crammed { 2 } else { 3 }]);
        }
    }
    out
}

#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    len: usize,
}
impl Bits {
    fn push(&mut self, bit: bool) {
        if self.len % 8 == 0 {
            self.bytes.push(0);
        }
        if bit {
            *self.bytes.last_mut().unwrap() |= 1 << (self.len % 8);
        }
        self.len += 1;
    }
    fn finish(mut self) -> Vec<u8> {
        pad4(&mut self.bytes);
        self.bytes
    }
}
fn pad4(bytes: &mut Vec<u8>) {
    bytes.resize(bytes.len().next_multiple_of(4), 0);
}
fn push_size(out: &mut Vec<u8>, size: usize) -> Result<()> {
    out.extend(
        u32::try_from(size)
            .map_err(|_| Error("binary field exceeds 32 bits"))?
            .to_le_bytes(),
    );
    Ok(())
}

type TrieImage = (Vec<u8>, BTreeMap<Vec<u8>, usize>);
fn trie(mut words: Vec<Vec<u8>>) -> Result<TrieImage> {
    words.sort_unstable();
    words.dedup();
    let mut ids = vec![0; words.len()];
    let mut next_id = 0;
    let mut louds = Bits::default();
    louds.push(true);
    louds.push(false);
    let mut terminals = Bits::default();
    terminals.push(false);
    let mut labels = vec![0];
    let mut queue = VecDeque::from([(0, words.len(), 0)]);
    while let Some((mut start, end, depth)) = queue.pop_front() {
        if start < end && words[start].len() == depth {
            start += 1;
        }
        while start < end {
            let byte = words[start][depth];
            let mut stop = start + 1;
            while stop < end && words[stop][depth] == byte {
                stop += 1;
            }
            louds.push(true);
            labels.push(byte);
            let terminal = words[start].len() == depth + 1;
            terminals.push(terminal);
            if terminal {
                ids[start] = next_id;
                next_id += 1;
            }
            queue.push_back((start, stop, depth + 1));
            start = stop;
        }
        louds.push(false);
    }
    let louds = louds.finish();
    let terminals = terminals.finish();
    let mut image = Vec::new();
    for size in [louds.len(), terminals.len(), 8, labels.len()] {
        push_size(&mut image, size)?;
    }
    image.extend(louds);
    image.extend(terminals);
    image.extend(labels);
    Ok((image, words.into_iter().zip(ids).collect()))
}

fn token_array(rows: Vec<Vec<u8>>) -> Result<Vec<u8>> {
    let mut bits = Bits::default();
    let mut data = Vec::new();
    for row in rows {
        bits.push(false);
        for _ in 4..row.len() {
            bits.push(true);
        }
        let padding = 4usize.saturating_sub(row.len());
        data.extend(row);
        data.resize(data.len() + padding, 0);
    }
    bits.push(false);
    let bits = bits.finish();
    let mut image = Vec::new();
    for size in [bits.len(), 4, 1, 0] {
        push_size(&mut image, size)?;
    }
    image.extend(bits);
    image.extend(data);
    Ok(image)
}
