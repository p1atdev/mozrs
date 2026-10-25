// Copyright 2026 mozrs contributors. Portions derived from Mozc; see LICENSE.
use crate::{
    Error, Result, ensure,
    reader::{Bits, Reader},
};

const NOT_TERMINAL: u32 = u32::MAX;

#[derive(Clone, Default)]
struct Node {
    parent: u32,
    begin: u32,
    end: u32,
    key_id: u32,
}

/// A validated traversal index over the upstream LOUDS representation.
/// Node indices are expanded on load; this is not a succinct-memory implementation.
pub(crate) struct Trie<'a> {
    labels: &'a [u8],
    nodes: Vec<Node>,
    terminals: Vec<u32>,
}

impl<'a> Trie<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let louds_len = r.size()?;
        let terminal_len = r.size()?;
        ensure(r.u32()? == 8, "unsupported trie label width")?;
        let count = r.size()?;
        ensure(
            count > 0 && louds_len % 4 == 0 && terminal_len % 4 == 0,
            "invalid trie sizes",
        )?;
        let mut bits = Bits::new(r.take(louds_len)?);
        let mut terminal_bits = Bits::new(r.take(terminal_len)?);
        let labels = r.take(count)?;
        ensure(r.rest.is_empty() && labels[0] == 0, "invalid trie labels")?;
        ensure(bits.next()? && !bits.next()?, "invalid LOUDS root")?;
        let mut nodes = vec![Node::default(); count];
        let mut terminals = Vec::new();
        let mut next_child = 1;
        for parent in 0..count {
            ensure(parent < next_child, "disconnected LOUDS node")?;
            let begin = next_child;
            while bits.next()? {
                ensure(next_child < count, "too many LOUDS edges")?;
                nodes[next_child].parent = parent as u32;
                next_child += 1;
            }
            ensure(
                labels[begin..next_child].windows(2).all(|p| p[0] < p[1]),
                "unsorted or duplicate trie edges",
            )?;
            nodes[parent].begin = begin as u32;
            nodes[parent].end = next_child as u32;
            nodes[parent].key_id = if terminal_bits.next()? {
                ensure(parent != 0, "terminal root is unsupported")?;
                let id = terminals.len() as u32;
                terminals.push(parent as u32);
                id
            } else {
                NOT_TERMINAL
            };
        }
        ensure(next_child == count, "inconsistent LOUDS edge count")?;
        bits.finish()?;
        terminal_bits.finish()?;
        Ok(Self {
            labels,
            nodes,
            terminals,
        })
    }

    pub fn len(&self) -> usize {
        self.terminals.len()
    }

    pub fn child(&self, parent: usize, label: u8) -> Option<usize> {
        let node = &self.nodes[parent];
        let begin = node.begin as usize;
        self.labels[begin..node.end as usize]
            .binary_search(&label)
            .ok()
            .map(|i| begin + i)
    }

    pub fn key_id(&self, node: usize) -> Option<usize> {
        let id = self.nodes[node].key_id;
        (id != NOT_TERMINAL).then_some(id as usize)
    }

    pub fn restore(&self, key_id: usize) -> Result<Vec<u8>> {
        let mut node = *self
            .terminals
            .get(key_id)
            .ok_or(Error("value trie ID out of bounds"))? as usize;
        let mut value = Vec::new();
        while node != 0 {
            value.push(self.labels[node]);
            node = self.nodes[node].parent as usize;
        }
        value.reverse();
        Ok(value)
    }
}

pub(crate) fn token_rows(data: &[u8], count: usize) -> Result<Vec<&[u8]>> {
    let mut r = Reader::new(data);
    let index_len = r.size()?;
    let base = r.size()?;
    let step = r.size()?;
    ensure(
        r.u32()? == 0 && index_len % 4 == 0 && base > 0 && step > 0,
        "invalid token-array header",
    )?;
    let mut bits = Bits::new(r.take(index_len)?);
    let mut rows = Vec::new();
    // One extra row is the upstream token-array sentinel.
    ensure(!bits.next()?, "invalid token-array index")?;
    for _ in 0..=count {
        let mut len = base;
        while bits.next()? {
            len = len
                .checked_add(step)
                .ok_or(Error("token-array length overflow"))?;
        }
        rows.push(r.take(len)?);
    }
    bits.finish()?;
    ensure(r.rest.is_empty(), "trailing token-array data")?;
    let sentinel = rows.pop().ok_or(Error("missing token sentinel"))?;
    ensure(
        sentinel.first() == Some(&0xff) && sentinel[1..].iter().all(|&b| b == 0),
        "invalid token sentinel",
    )?;
    Ok(rows)
}
