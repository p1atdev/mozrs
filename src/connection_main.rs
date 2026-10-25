// Copyright 2026 mozrs contributors. BSD-3-Clause; see LICENSE.
use mozrs::connection::{Connection, unknown_pos_id};
use std::{
    env,
    error::Error,
    fs,
    io::{self, BufReader, Write},
};

const USAGE: &str = "Usage: mozrs-build-connection RAW_CONNECTION.txt ID.def SPECIAL_POS.def OUTPUT.data\n       mozrs-build-connection --dump-costs CONNECTION.data\nBuild pinned OSS resolution-1 connection.data without C++/Bazel.\n--dump-costs writes the decoded matrix as row-major little-endian u16 to stdout.";
fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() == 2 && args[0] == "--dump-costs" {
        let matrix = Connection::from_bytes(&fs::read(&args[1])?)?;
        let mut output = io::BufWriter::new(io::stdout().lock());
        for &cost in matrix.costs() {
            output.write_all(&cost.to_le_bytes())?;
        }
        output.flush()?;
        return Ok(());
    }
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    if args.len() != 4 {
        return Err(USAGE.into());
    }
    let ids = fs::read_to_string(&args[1])?;
    let special = fs::read_to_string(&args[2])?;
    let connection =
        Connection::from_text(BufReader::new(fs::File::open(&args[0])?), &ids, &special)?;
    let unknown = unknown_pos_id(&ids)?;
    let image = connection.to_bytes()?;
    let decoded = Connection::from_bytes(&image)?;
    if decoded.costs() != connection.costs() {
        return Err("connection roundtrip mismatch".into());
    }
    fs::write(&args[3], &image)?;
    println!(
        "{}",
        serde_json::json!({"output": args[3].to_string_lossy(), "pos_size": connection.size(),
        "unknown_pos_id": unknown, "resolution": 1, "bytes": image.len(), "upstream_revision": mozrs::UPSTREAM_REVISION})
    );
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("mozrs-build-connection: {e}");
        std::process::exit(1);
    }
}
