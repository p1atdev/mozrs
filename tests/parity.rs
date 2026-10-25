use mozrs::{Dictionary, Token};
use serde::Deserialize;
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

const IMAGE: &[u8] = include_bytes!("fixtures/synthetic.data");
const EXPECTED: &str = include_str!("fixtures/synthetic.jsonl");

#[derive(Deserialize)]
struct Case {
    query: String,
    mode: String,
    tokens: Vec<Token>,
}

#[test]
fn every_fixture_lookup_matches_upstream_cpp_including_order_and_duplicates() {
    let dict = Dictionary::from_bytes(IMAGE).unwrap();
    assert_eq!(dict.validate().unwrap(), 620);
    let mut cases = 0;
    for line in EXPECTED.lines() {
        let case: Case = serde_json::from_str(line).unwrap();
        let actual = match case.mode.as_str() {
            "exact" => dict.lookup_exact(&case.query),
            "prefix" => dict.lookup_prefix(&case.query),
            _ => panic!("unexpected fixture mode"),
        }
        .unwrap();
        assert_eq!(actual, case.tokens, "{} {:?}", case.mode, case.query);
        cases += 1;
    }
    assert!(
        cases > 1200,
        "fixture must cover all synthetic readings and extensions"
    );
}

#[test]
fn missing_keys_empty_input_and_prefix_direction_are_explicit() {
    let dict = Dictionary::from_bytes(IMAGE).unwrap();
    for key in ["", "不存在", "\0", "がつこう"] {
        assert!(dict.lookup_exact(key).unwrap().is_empty());
        assert!(dict.lookup_prefix(key).unwrap().is_empty());
    }
    assert!(dict.lookup_exact("あいうえお").unwrap().is_empty());
    let tokens = dict.lookup_prefix("あいうえお").unwrap();
    assert_eq!(tokens.len(), 7);
    assert!(tokens.iter().all(|t| "あいうえお".starts_with(&t.key)));
    assert!(
        dict.lookup_prefix("あ")
            .unwrap()
            .iter()
            .all(|t| t.key == "あ")
    );
}

#[test]
fn cli_outputs_json_and_reports_bad_arguments_and_missing_files() {
    let bin = env!("CARGO_BIN_EXE_mozrs");
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/synthetic.data");
    let output = Command::new(bin)
        .args([path, "exact", "ascii", "不存在"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines: Vec<Case> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].tokens[0].value, "A\tB\n\"\\");
    assert!(lines[1].tokens.is_empty());
    for args in [
        vec![],
        vec![path, "unknown"],
        vec!["missing-file.data", "exact", "a"],
        vec![path, "check", "extra"],
    ] {
        let output = Command::new(bin).args(args).output().unwrap();
        assert!(!output.status.success());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn cli_rejects_malformed_dictionary_before_emitting_results() {
    let path = std::env::temp_dir().join(format!("mozrs-malformed-{}.data", std::process::id()));
    fs::write(&path, &IMAGE[..IMAGE.len() - 1]).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mozrs"))
        .arg(&path)
        .args(["exact", "あ"])
        .output()
        .unwrap();
    fs::remove_file(path).unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn cli_streams_one_response_per_stdin_line_including_empty_queries() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/synthetic.data");
    let mut child = Command::new(env!("CARGO_BIN_EXE_mozrs"))
        .args([path, "prefix"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all("あいうえお\r\n\n不存在\n".as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let lines: Vec<Case> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].tokens.len(), 7);
    assert!(lines[1].query.is_empty() && lines[1].tokens.is_empty());
    assert!(lines[2].tokens.is_empty());
}
