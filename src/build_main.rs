// Copyright 2026 mozrs contributors. BSD-3-Clause; see LICENSE.
use mozrs::{
    Token,
    builder::{self, BuildOptions},
    input::{self, PosIds},
};
use std::{
    env,
    error::Error,
    ffi::OsString,
    fs,
    io::{BufRead, BufReader},
    path::PathBuf,
};

const USAGE: &str = "Usage: mozrs-build --output FILE [--format tsv|jsonl] [--id-def FILE --special-pos FILE] [--corrections FILE] [--min-key-length-for-small-cost N] INPUT...\n\
TSV: key<TAB>lid<TAB>rid<TAB>cost<TAB>value[<TAB>label], normalized like Mozc.\n\
JSONL: one prepared Token per line, without normalization. Default cost threshold: 6.\n\
Writes standalone dictionary.data. C++ and Bazel are not needed.";

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let mut output = None;
    let mut id_def = None;
    let mut special_pos = None;
    let mut corrections = None;
    let mut format = OsString::from("tsv");
    let mut options = BuildOptions::default();
    let mut inputs = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--" {
            inputs.extend(args.map(PathBuf::from));
            break;
        }
        if arg == "--help" || arg == "-h" {
            println!("{USAGE}");
            return Ok(());
        }
        let mut value = || args.next().ok_or("option requires a value");
        match arg.to_str() {
            Some("--output") => output = Some(PathBuf::from(value()?)),
            Some("--id-def") => id_def = Some(PathBuf::from(value()?)),
            Some("--special-pos") => special_pos = Some(PathBuf::from(value()?)),
            Some("--corrections") => corrections = Some(PathBuf::from(value()?)),
            Some("--format") => format = value()?,
            Some("--min-key-length-for-small-cost") => {
                options.min_key_length_for_small_cost =
                    value()?.to_str().ok_or("invalid threshold")?.parse()?;
            }
            Some(s) if s.starts_with('-') => return Err(format!("unknown option: {s}").into()),
            _ => inputs.push(PathBuf::from(arg)),
        }
    }
    let output = output.ok_or(USAGE)?;
    if inputs.is_empty() {
        return Err(USAGE.into());
    }
    if format != "tsv" && format != "jsonl" {
        return Err("format must be tsv or jsonl".into());
    }
    if format == "jsonl" && (id_def.is_some() || special_pos.is_some() || corrections.is_some()) {
        return Err(
            "JSONL is prepared input; POS definitions/corrections apply only to TSV".into(),
        );
    }
    let pos = match (id_def, special_pos) {
        (Some(id), Some(special)) => Some(PosIds::from_definitions(
            &fs::read_to_string(id)?,
            &fs::read_to_string(special)?,
        )?),
        (None, None) => None,
        _ => return Err("--id-def and --special-pos must be supplied together".into()),
    };
    let mut tokens = Vec::new();
    for path in &inputs {
        for (i, line) in BufReader::new(fs::File::open(path)?).lines().enumerate() {
            let line = line?;
            let token: Result<Token, Box<dyn Error>> = if format == "jsonl" {
                serde_json::from_str(&line).map_err(Into::into)
            } else {
                input::parse_tsv_line(&line, pos).map_err(Into::into)
            };
            tokens.push(token.map_err(|e| format!("{}:{}: {e}", path.display(), i + 1))?);
        }
    }
    if let Some(path) = corrections {
        input::add_reading_corrections(&mut tokens, &fs::read_to_string(path)?)?;
    }
    let token_count = tokens.len();
    let image = builder::build(tokens, &options)?;
    // Validate before opening the output, so invalid input cannot truncate it.
    let dictionary = mozrs::Dictionary::from_bytes(&image)?;
    if dictionary.validate()? != token_count {
        return Err("built token count mismatch".into());
    }
    let key_count = dictionary.key_count();
    fs::write(&output, &image)?;
    println!(
        "{}",
        serde_json::json!({"output": output.to_string_lossy(), "keys": key_count, "tokens": token_count,
        "bytes": image.len(), "upstream_revision": mozrs::UPSTREAM_REVISION})
    );
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("mozrs-build: {error}");
        std::process::exit(1);
    }
}
