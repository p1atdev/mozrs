// Copyright 2026 mozrs contributors. BSD-3-Clause; see LICENSE.
use mozrs::{Dictionary, connection::Connection, converter::Converter};
use std::{
    env,
    error::Error,
    fs,
    io::{self, BufRead, Write},
};

const USAGE: &str = "Usage: mozrs-convert DICTIONARY.data CONNECTION.data UNKNOWN_POS_ID [QUERY ...]\nWith no QUERY, read one UTF-8 query per stdin line. Output one JSON best path per line.\nBasic word-cost + connection-cost search, not full Mozc conversion.\nUnknown scalars are preserved; no normalization, segmentation, rewriters or learning.";
fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let path = args.next().ok_or(USAGE)?;
    if path == "--help" || path == "-h" {
        println!("{USAGE}");
        return Ok(());
    }
    let connection_path = args.next().ok_or(USAGE)?;
    let unknown: u16 = args
        .next()
        .ok_or(USAGE)?
        .to_str()
        .ok_or("invalid unknown POS ID")?
        .parse()?;
    let queries = args
        .map(|q| q.into_string().map_err(|_| "query must be UTF-8"))
        .collect::<Result<Vec<_>, _>>()?;
    let image = fs::read(path)?;
    let dictionary = Dictionary::from_bytes(&image)?;
    let connection = Connection::from_bytes(&fs::read(connection_path)?)?;
    let converter = Converter::new(&dictionary, &connection, unknown)?;
    let mut out = io::BufWriter::new(io::stdout().lock());
    let mut convert = |query: &str| -> Result<(), Box<dyn Error>> {
        serde_json::to_writer(&mut out, &converter.convert(query)?)?;
        writeln!(out)?;
        Ok(())
    };
    if queries.is_empty() {
        for query in io::stdin().lock().lines() {
            convert(&query?)?;
        }
    } else {
        for query in queries {
            convert(&query)?;
        }
    }
    out.flush()?;
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("mozrs-convert: {e}");
        std::process::exit(1);
    }
}
