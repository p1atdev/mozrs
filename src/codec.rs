// Copyright 2026 mozrs contributors. Portions derived from Mozc; see LICENSE.
use crate::{Error, Result, ensure, reader::Reader};

// The key transform is an involution: applying it twice restores the input.
pub(crate) fn transform_key(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            let code = c as u32;
            let mapped = match code {
                1..=0x1f => code + 0x3040,
                0x3041..=0x305f => code - 0x3040,
                0x40..=0x75 => code + 0x3020,
                0x3060..=0x3095 => code - 0x3020,
                0x76..=0x77 => code + (0x30fb - 0x76),
                0x30fb..=0x30fc => code - (0x30fb - 0x76),
                _ => code,
            };
            char::from_u32(mapped).unwrap()
        })
        .collect()
}

pub(crate) fn encode_value(input: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for c in input.chars().map(u32::from) {
        let high = (c >> 8) as u8;
        let low = c as u8;
        match c {
            0x3041..=0x3094 => out.push((c - 0x3041 + 0x4b) as u8),
            0x30a1..=0x30fc => out.push((c - 0x30a1 + 0x9f) as u8),
            0..=0xff => out.extend([0xfc, low]),
            0x100..=0xffff if low == 0 => out.extend([0xfd, high]),
            0x4e00..=0x97ff => out.extend([((c - 0x4e00) >> 8) as u8 + 1, low]),
            0x10000.. => {
                let flags = (c >> 16) as u8
                    | if high == 0 { 0x80 } else { 0 }
                    | if low == 0 { 0x40 } else { 0 };
                out.extend([0xff, flags]);
                if high != 0 {
                    out.push(high);
                }
                if low != 0 {
                    out.push(low);
                }
            }
            _ => out.extend([0xfe, high, low]),
        }
    }
    out
}

pub(crate) fn decode_value(data: &[u8]) -> Result<String> {
    let mut r = Reader::new(data);
    let mut out = String::new();
    while !r.rest.is_empty() {
        let tag = r.byte()?;
        let code = match tag {
            1..=0x4a => ((u32::from(tag) - 1) << 8) + 0x4e00 + u32::from(r.byte()?),
            0x4b..=0x9e => 0x3041 + u32::from(tag) - 0x4b,
            0x9f..=0xfa => 0x30a1 + u32::from(tag) - 0x9f,
            0xfc => u32::from(r.byte()?),
            0xfd => u32::from(r.byte()?) << 8,
            0xfe => (u32::from(r.byte()?) << 8) | u32::from(r.byte()?),
            0xff => {
                let flags = r.byte()?;
                ensure(flags & 0x20 == 0, "reserved Unicode flag")?;
                let mut code = u32::from(flags & 0x1f) << 16;
                if flags & 0x80 == 0 {
                    code |= u32::from(r.byte()?) << 8;
                }
                if flags & 0x40 == 0 {
                    code |= u32::from(r.byte()?);
                }
                ensure(code >= 0x10000, "invalid supplementary Unicode encoding")?;
                code
            }
            _ => return Err(Error("reserved value encoding")),
        };
        out.push(char::from_u32(code).ok_or(Error("invalid Unicode scalar value"))?);
    }
    Ok(out)
}

pub(crate) fn katakana(input: &str) -> String {
    let mut chars = input.chars().peekable();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        // Upstream data/preedit/hiragana-katakana.tsv includes this digraph,
        // but does not map small ka/ke or iteration marks.
        if c == 'う' && chars.peek() == Some(&'゛') {
            chars.next();
            out.push('ヴ');
            continue;
        }
        let code = c as u32;
        out.push(if (0x3041..=0x3094).contains(&code) {
            char::from_u32(code + 0x60).unwrap()
        } else {
            c
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::decode_value;

    #[test]
    fn malformed_value_sequences_return_errors() {
        for bytes in [
            &[0xfc][..],
            &[0xfd][..],
            &[0xfe, 0xd8, 0][..],
            &[0xff][..],
            &[0xff, 0x01][..],
            &[0xff, 0x91, 0x01][..],
            &[0xff, 0xe1][..],
            &[0xfb][..],
            &[0][..],
        ] {
            assert!(decode_value(bytes).is_err(), "{bytes:?}");
        }
    }
}
