use mozrs::Dictionary;
use serde::Serialize;
use std::{
    env,
    error::Error,
    fs,
    io::{self, BufRead, Write},
    process::ExitCode,
};

const USAGE: &str = "Usage: mozrs DICTIONARY.data exact|prefix [QUERY ...]\n       mozrs DICTIONARY.data check\n\nWith no QUERY arguments, read one UTF-8 query per stdin line.\nOutput: one JSON object per query. prefix finds dictionary readings that are\nprefixes of the query, not predictive completions. check validates all tokens.\nSupports pinned Mozc OSS dictionary.data; not mozc.data or Google IME data.";

#[derive(Serialize)]
struct Response<'a> {
    query: &'a str,
    mode: &'a str,
    tokens: Vec<mozrs::Token>,
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let Some(path) = args.next() else {
        return Err(USAGE.into());
    };
    if path == "--help" || path == "-h" {
        println!("{USAGE}");
        return Ok(());
    }
    let mode = args
        .next()
        .ok_or(USAGE)?
        .into_string()
        .map_err(|_| "mode must be UTF-8")?;
    if !matches!(mode.as_str(), "exact" | "prefix" | "check") {
        return Err(USAGE.into());
    }
    let queries = args
        .map(|s| s.into_string().map_err(|_| "query must be UTF-8"))
        .collect::<Result<Vec<_>, _>>()?;
    if mode == "check" && !queries.is_empty() {
        return Err(USAGE.into());
    }
    let image = fs::read(path)?;
    let dict = Dictionary::from_bytes(&image)?;
    // CLI rejects corruption anywhere in the file before producing results.
    let token_count = dict.validate()?;
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    if mode == "check" {
        serde_json::to_writer(
            &mut stdout,
            &serde_json::json!({"keys": dict.key_count(), "tokens": token_count, "upstream_revision": mozrs::UPSTREAM_REVISION}),
        )?;
        writeln!(stdout)?;
    } else {
        let mut lookup = |query: &str| -> Result<(), Box<dyn Error>> {
            let tokens = if mode == "exact" {
                dict.lookup_exact(query)?
            } else {
                dict.lookup_prefix(query)?
            };
            serde_json::to_writer(
                &mut stdout,
                &Response {
                    query,
                    mode: &mode,
                    tokens,
                },
            )?;
            writeln!(stdout)?;
            Ok(())
        };
        if queries.is_empty() {
            for line in io::stdin().lock().lines() {
                lookup(&line?)?;
            }
        } else {
            for query in queries {
                lookup(&query)?;
            }
        }
    }
    stdout.flush()?;
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mozrs: {e}");
            ExitCode::FAILURE
        }
    }
}
