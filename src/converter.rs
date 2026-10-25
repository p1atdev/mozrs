// Copyright 2026 mozrs contributors. Portions derived from Mozc; see LICENSE.
//! One best path over dictionary tokens, without Mozc segmentation/rewriters.
use crate::{Dictionary, Error, Result, Token, connection::Connection, ensure};
use serde::{Deserialize, Serialize};

pub const UNKNOWN_WORD_COST: u16 = 32767;
pub const MAX_INPUT_BYTES: usize = 4096;
const MAX_NODES: usize = 200_000;
const MAX_EDGES: usize = 10_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertedToken {
    /// UTF-8 byte offsets in the unchanged query, not grapheme/character indices.
    pub start: usize,
    pub end: usize,
    #[serde(flatten)]
    pub token: Token,
    pub unknown: bool,
    pub transition_cost: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversion {
    pub query: String,
    pub value: String,
    pub cost: u64,
    pub eos_cost: u16,
    /// Word nodes, not Mozc's editable conversion segments.
    pub tokens: Vec<ConvertedToken>,
}

pub struct Converter<'a, 'data> {
    dictionary: &'a Dictionary<'data>,
    connection: &'a Connection,
    unknown_pos: u16,
}
struct Node {
    token: ConvertedToken,
    cost: u64,
    previous: Option<usize>,
}

impl<'a, 'data> Converter<'a, 'data> {
    /// Validate all dictionary POS IDs against this matrix once at construction.
    /// Callers must supply data from the same POS definitions; IDs alone cannot
    /// detect a semantically unrelated matrix of the same size.
    pub fn new(
        dictionary: &'a Dictionary<'data>,
        connection: &'a Connection,
        unknown_pos: u16,
    ) -> Result<Self> {
        ensure(
            unknown_pos != 0 && usize::from(unknown_pos) < connection.size(),
            "invalid unknown POS ID",
        )?;
        dictionary.validate_pos_limit(connection.size())?;
        Ok(Self {
            dictionary,
            connection,
            unknown_pos,
        })
    }

    /// Minimize decoded word costs + transitions, including BOS/EOS ID 0.
    /// At every Unicode scalar boundary append one unchanged-scalar fallback
    /// (cost 32767), even when dictionary tokens exist. No normalization occurs.
    /// Ties retain the first predecessor in insertion order: ascending start
    /// byte, dictionary prefix/storage order, then fallback. EOS uses that order.
    /// Limits return errors, never a pruned or approximate result.
    pub fn convert(&self, query: &str) -> Result<Conversion> {
        ensure(
            query.len() <= MAX_INPUT_BYTES,
            "conversion input exceeds 4096 UTF-8 bytes",
        )?;
        ensure(!query.contains('\0'), "NUL in conversion input")?;
        if query.is_empty() {
            return Ok(Conversion {
                query: String::new(),
                value: String::new(),
                cost: 0,
                eos_cost: 0,
                tokens: vec![],
            });
        }
        let mut nodes: Vec<Node> = Vec::new();
        let mut ending: Vec<Vec<usize>> = vec![Vec::new(); query.len() + 1];
        let mut edges = 0usize;
        for (start, c) in query.char_indices() {
            let mut candidates: Vec<_> = self
                .dictionary
                .lookup_prefix(&query[start..])?
                .into_iter()
                .map(|t| (t, false))
                .collect();
            candidates.push((
                Token {
                    key: c.to_string(),
                    value: c.to_string(),
                    cost: UNKNOWN_WORD_COST,
                    lid: self.unknown_pos,
                    rid: self.unknown_pos,
                    attributes: 0,
                },
                true,
            ));
            ensure(
                nodes.len() + candidates.len() <= MAX_NODES,
                "conversion lattice exceeds node limit",
            )?;
            for (token, unknown) in candidates {
                let end = start + token.key.len();
                let (cost, previous, transition_cost) = if start == 0 {
                    let edge = self.connection.cost(0, token.lid)?;
                    (u64::from(edge) + u64::from(token.cost), None, edge)
                } else {
                    edges += ending[start].len();
                    ensure(edges <= MAX_EDGES, "conversion lattice exceeds edge limit")?;
                    let mut best = None;
                    for &id in &ending[start] {
                        let previous = &nodes[id];
                        let edge = self.connection.cost(previous.token.token.rid, token.lid)?;
                        let cost = previous.cost + u64::from(edge) + u64::from(token.cost);
                        if best
                            .as_ref()
                            .is_none_or(|&(best_cost, _, _)| cost < best_cost)
                        {
                            best = Some((cost, Some(id), edge));
                        }
                    }
                    best.ok_or(Error("no conversion path"))?
                };
                ending[end].push(nodes.len());
                nodes.push(Node {
                    token: ConvertedToken {
                        start,
                        end,
                        token,
                        unknown,
                        transition_cost,
                    },
                    cost,
                    previous,
                });
            }
        }
        let mut best = None;
        for &id in &ending[query.len()] {
            let edge = self.connection.cost(nodes[id].token.token.rid, 0)?;
            let cost = nodes[id].cost + u64::from(edge);
            if best
                .as_ref()
                .is_none_or(|&(best_cost, _, _)| cost < best_cost)
            {
                best = Some((cost, id, edge));
            }
        }
        let (cost, id, eos_cost) = best.ok_or(Error("no conversion path"))?;
        let mut path = Vec::new();
        let mut cursor = Some(id);
        while let Some(id) = cursor {
            path.push(nodes[id].token.clone());
            cursor = nodes[id].previous;
        }
        path.reverse();
        let value = path.iter().map(|n| n.token.value.as_str()).collect();
        Ok(Conversion {
            query: query.into(),
            value,
            cost,
            eos_cost,
            tokens: path,
        })
    }
}
