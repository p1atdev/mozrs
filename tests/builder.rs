use mozrs::{
    Dictionary, Token,
    builder::{BuildOptions, build},
    input::{self, PosIds},
};
use std::{fs, process::Command};

fn tokens(jsonl: &str) -> Vec<Token> {
    jsonl
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn simple(key: &str, value: &str) -> Token {
    Token {
        key: key.into(),
        value: value.into(),
        lid: 1,
        rid: 2,
        cost: 9999,
        attributes: 0,
    }
}

#[test]
fn original_synthetic_inputs_reproduce_cpp_bytes() {
    let input = tokens(include_str!("fixtures/synthetic-input.jsonl"));
    let image = build(
        input,
        &BuildOptions {
            min_key_length_for_small_cost: 1,
        },
    )
    .unwrap();
    assert_eq!(image, include_bytes!("fixtures/synthetic.data"));
}

#[test]
fn tsv_pos_normalization_and_corrections_match_cpp_loader_and_builder() {
    let pos = PosIds::from_definitions(
        include_str!("fixtures/loader-id.def"),
        include_str!("fixtures/loader-special-pos.def"),
    )
    .unwrap();
    assert_eq!(
        pos,
        PosIds {
            zip_code: 10,
            isolated_word: 11
        }
    );
    let mut input: Vec<_> = include_str!("fixtures/loader.tsv")
        .lines()
        .map(|line| input::parse_tsv_line(line, Some(pos)).unwrap())
        .collect();
    input::add_reading_corrections(&mut input, include_str!("fixtures/loader-corrections.tsv"))
        .unwrap();
    assert_eq!(input, tokens(include_str!("fixtures/loader-input.jsonl")));
    let image = build(input, &BuildOptions::default()).unwrap();
    assert_eq!(image, include_bytes!("fixtures/loader.data"));
    let dict = Dictionary::from_bytes(&image).unwrap();
    assert_eq!(dict.lookup_exact("まちがい").unwrap().len(), 2);
    assert_eq!(dict.lookup_exact("が").unwrap()[0].cost, 2502);
    assert_eq!(
        dict.lookup_exact("じゅんじょ")
            .unwrap()
            .iter()
            .map(|t| t.cost)
            .collect::<Vec<_>>(),
        [500, 100]
    );
    assert!(dict.lookup_exact("不存在").unwrap().is_empty());
}

#[test]
fn small_cost_is_lossy_only_when_eligible_and_can_be_disabled() {
    let input = vec![
        simple("ながいよみそのいち", "単独"),
        simple("ながいよみそのに", "同音一"),
        simple("ながいよみそのに", "同音二"),
        simple("ながいよみそのさん", "多読"),
        simple("ながいよみそのよん", "多読"),
        simple("みじかい", "短"),
    ];
    for threshold in [6, usize::MAX] {
        let image = build(
            input.clone(),
            &BuildOptions {
                min_key_length_for_small_cost: threshold,
            },
        )
        .unwrap();
        let dict = Dictionary::from_bytes(&image).unwrap();
        for t in &input {
            let expected = if threshold == 6 && t.value == "単独" {
                9984
            } else {
                9999
            };
            assert_eq!(dict.lookup_exact(&t.key).unwrap()[0].cost, expected);
        }
    }
}

#[test]
fn empty_kana_only_and_maximum_depth_dictionaries_work() {
    let image = build(vec![], &BuildOptions::default()).unwrap();
    let dict = Dictionary::from_bytes(&image).unwrap();
    assert_eq!(dict.validate().unwrap(), 0);
    assert!(dict.lookup_prefix("あ").unwrap().is_empty());
    let key = "あ".repeat(256);
    let image = build(
        vec![simple(&key, &key), simple("かな", "カナ")],
        &BuildOptions::default(),
    )
    .unwrap();
    let dict = Dictionary::from_bytes(&image).unwrap();
    assert_eq!(dict.validate().unwrap(), 2);
    assert_eq!(dict.lookup_exact(&key).unwrap()[0].value, key);
    assert_eq!(dict.lookup_exact("かな").unwrap()[0].value, "カナ");
}

#[test]
fn invalid_prepared_tokens_are_rejected() {
    let valid = simple("よみ", "表記");
    let mut invalid = vec![
        simple("", "表記"),
        simple("よみ", ""),
        simple("a\0", "表記"),
        simple("よみ", "v\0"),
        simple(&"あ".repeat(257), "表記"),
        simple("よみ", &"あ".repeat(257)),
    ];
    for field in 0..4 {
        let mut t = valid.clone();
        match field {
            0 => t.cost = 32768,
            1 => t.lid = 4096,
            2 => t.rid = 4096,
            _ => t.attributes = 2,
        }
        invalid.push(t);
    }
    for t in invalid {
        assert!(build(vec![t], &BuildOptions::default()).is_err());
    }
}

#[test]
fn malformed_tsv_pos_and_corrections_return_errors() {
    for line in [
        "",
        "a\t1\t2\t3",
        "a\t-1\t2\t3\tb",
        "a\t1\t2\t32768\tb",
        "a\t1\t2\t3\tb\tUNKNOWN",
        "a\t1\t2\t3\tb\tZIP_CODE",
    ] {
        assert!(input::parse_tsv_line(line, None).is_err());
    }
    for defs in ["", "x y", "4096 y", "1", "1 a extra"] {
        assert!(PosIds::from_definitions(defs, "特殊,郵便番号\n特殊,短縮よみ").is_err());
    }
    assert!(PosIds::from_definitions("4095 a", "特殊,郵便番号").is_err());
    for correction in ["malformed", "\treading", "表記\t"] {
        assert!(
            input::add_reading_corrections(&mut vec![simple("よみ", "表記")], correction).is_err()
        );
    }
    let mut t = simple("よみ", "表記");
    t.cost = 32767;
    assert!(input::add_reading_corrections(&mut vec![t], "表記\tごよみ").is_err());
}

#[test]
fn builder_cli_writes_cpp_identical_image_and_preserves_output_on_invalid_input() {
    let directory = std::env::temp_dir().join(format!("mozrs-builder-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let output = directory.join("dictionary.data");
    let bin = env!("CARGO_BIN_EXE_mozrs-build");
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/synthetic-input.jsonl"
    );
    let result = Command::new(bin)
        .args([
            "--format",
            "jsonl",
            "--min-key-length-for-small-cost",
            "1",
            "--output",
        ])
        .arg(&output)
        .arg(fixture)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        fs::read(&output).unwrap(),
        include_bytes!("fixtures/synthetic.data")
    );
    let invalid = directory.join("bad.tsv");
    for data in [&b"bad\n"[..], &b"\xff\n"[..], &b"a\t1\t2\t32768\tb\n"[..]] {
        fs::write(&invalid, data).unwrap();
        let result = Command::new(bin)
            .arg("--output")
            .arg(&output)
            .arg(&invalid)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        assert_eq!(
            fs::read(&output).unwrap(),
            include_bytes!("fixtures/synthetic.data")
        );
    }
    for args in [vec![], vec!["--output"], vec!["--unknown"]] {
        assert!(
            !Command::new(bin)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    fs::remove_dir_all(directory).unwrap();
}
