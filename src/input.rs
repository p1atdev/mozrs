// Copyright 2026 mozrs contributors. Portions derived from Mozc; see LICENSE.
//! The parts of TextDictionaryLoader needed for the OSS system dictionary.
use crate::{Error, Result, Token, ensure};
use std::{collections::HashMap, sync::OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PosIds {
    pub zip_code: u16,
    pub isolated_word: u16,
}
impl PosIds {
    /// Resolve the two literal features used by the pinned Zipcode and
    /// IsolatedWord rules. This is not a general POS regex matcher.
    pub fn from_definitions(id_def: &str, special_pos: &str) -> Result<Self> {
        let mut ids = Vec::new();
        for line in definition_lines(id_def) {
            let mut columns = line.split_whitespace();
            let id = columns
                .next()
                .ok_or(Error("missing POS ID"))?
                .parse::<u16>()
                .map_err(|_| Error("invalid POS ID"))?;
            let feature = columns.next().ok_or(Error("missing POS feature"))?;
            ensure(
                columns.next().is_none() && id <= 4095,
                "invalid POS definition",
            )?;
            ids.push((feature, id));
        }
        let mut next = ids
            .iter()
            .map(|(_, id)| *id)
            .max()
            .ok_or(Error("empty POS definitions"))?;
        for feature in definition_lines(special_pos) {
            next += 1; // previous IDs are bounded by the 12-bit format
            ensure(next <= 4095, "special POS ID exceeds 12 bits")?;
            ids.push((feature, next));
        }
        let find = |prefix: &str| {
            ids.iter()
                .filter(|(feature, _)| feature.starts_with(prefix))
                .map(|(_, id)| *id)
                .min()
                .ok_or(Error("required special POS is missing"))
        };
        Ok(Self {
            zip_code: find("特殊,郵便番号")?,
            isolated_word: find("特殊,短縮よみ")?,
        })
    }
}
fn definition_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

/// Apply exactly Mozc's voiced-sound table; this is not Unicode NFC/NFKC.
pub fn normalize_voiced_sound(input: &str) -> String {
    static RULES: OnceLock<HashMap<(char, char), char>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        include_str!("normalize-voiced-sound.tsv")
            .lines()
            .map(|line| {
                let (from, to) = line.split_once('\t').unwrap();
                let mut chars = from.chars();
                (
                    (chars.next().unwrap(), chars.next().unwrap()),
                    to.chars().next().unwrap(),
                )
            })
            .collect()
    });
    let mut chars = input.chars().peekable();
    let mut out = String::new();
    while let Some(first) = chars.next() {
        if let Some(mapped) = chars.peek().and_then(|&second| rules.get(&(first, second))) {
            out.push(*mapped);
            chars.next();
        } else {
            out.push(first);
        }
    }
    out
}

/// Parse `key TAB lid TAB rid TAB cost TAB value [TAB label]`.
/// ZIP_CODE and ENGLISH need POS definitions. Values are not whitespace-trimmed.
pub fn parse_tsv_line(line: &str, pos: Option<PosIds>) -> Result<Token> {
    // Matches upstream's SkipEmpty delimiter policy.
    let columns: Vec<_> = line.split('\t').filter(|s| !s.is_empty()).collect();
    ensure(columns.len() >= 5, "expected at least five TSV columns")?;
    let number = |s: &str| {
        s.parse::<u16>()
            .map_err(|_| Error("invalid unsigned TSV number"))
    };
    let mut token = Token {
        key: normalize_voiced_sound(columns[0]),
        value: normalize_voiced_sound(columns[4]),
        lid: number(columns[1])?,
        rid: number(columns[2])?,
        cost: number(columns[3])?,
        attributes: 0,
    };
    if let Some(label) = columns.get(5) {
        if label.starts_with("SPELLING_CORRECTION") {
            token.attributes = 1;
        } else if label.starts_with("ZIP_CODE") {
            token.lid = pos
                .ok_or(Error("ZIP_CODE requires POS definitions"))?
                .zip_code;
            token.rid = token.lid;
        } else if label.starts_with("ENGLISH") {
            token.lid = pos
                .ok_or(Error("ENGLISH requires POS definitions"))?
                .isolated_word;
            token.rid = token.lid;
        } else {
            return Err(Error("unknown TSV label"));
        }
    }
    ensure(token.cost <= 32767, "cost exceeds 15 bits")?;
    ensure(
        token.lid <= 4095 && token.rid <= 4095,
        "POS ID exceeds 12 bits",
    )?;
    Ok(token)
}

/// Insert reading corrections, preserving the pinned loader's tie behavior.
/// Each line is `value TAB wrong-reading TAB correct-reading`; the third field
/// is informational. New entries inherit the most costly original token for
/// that value, add 2302 to cost, and have ordinary (0) attributes. Corrections
/// are not normalized, and duplicates in the correction file remain duplicates.
pub fn add_reading_corrections(tokens: &mut Vec<Token>, corrections: &str) -> Result<()> {
    tokens.sort_by(|a, b| a.value.cmp(&b.value).then_with(|| a.key.cmp(&b.key)));
    let mut appended = Vec::new();
    for line in corrections
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let columns: Vec<_> = line.split('\t').collect();
        ensure(
            columns.len() >= 2,
            "expected value and wrong reading in correction",
        )?;
        let (value, key) = (columns[0], columns[1]);
        ensure(
            !value.is_empty() && !key.is_empty(),
            "empty reading correction field",
        )?;
        let begin = tokens.partition_point(|t| t.value.as_str() < value);
        let end = tokens.partition_point(|t| t.value.as_str() <= value);
        let candidates = &tokens[begin..end];
        if candidates.iter().any(|t| t.key == key) {
            continue;
        }
        let Some(mut best) = candidates.first() else {
            continue;
        };
        for token in &candidates[1..] {
            if token.cost > best.cost {
                best = token;
            }
        }
        let cost = best
            .cost
            .checked_add(2302)
            .filter(|c| *c <= 32767)
            .ok_or(Error("reading correction cost exceeds 15 bits"))?;
        appended.push(Token {
            key: key.to_owned(),
            value: value.to_owned(),
            cost,
            lid: best.lid,
            rid: best.rid,
            attributes: 0,
        });
    }
    tokens.extend(appended);
    Ok(())
}
