use mozrs::{
    Dictionary, Token,
    builder::{BuildOptions, build},
    connection::Connection,
    converter::{Conversion, Converter, MAX_INPUT_BYTES, UNKNOWN_WORD_COST},
};
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

fn token(key: &str, value: &str, cost: u16, pos: u16) -> Token {
    Token {
        key: key.into(),
        value: value.into(),
        cost,
        lid: pos,
        rid: pos,
        attributes: 0,
    }
}
fn image(tokens: Vec<Token>) -> Vec<u8> {
    build(tokens, &BuildOptions::default()).unwrap()
}
fn check_result(result: &Conversion, matrix: &Connection) {
    let mut end = 0;
    let mut rid = 0;
    let mut cost = 0u64;
    for t in &result.tokens {
        assert_eq!(t.start, end);
        assert_eq!(&result.query[t.start..t.end], t.token.key);
        assert_eq!(t.transition_cost, matrix.cost(rid, t.token.lid).unwrap());
        cost += u64::from(t.transition_cost) + u64::from(t.token.cost);
        end = t.end;
        rid = t.token.rid;
    }
    assert_eq!(end, result.query.len());
    assert_eq!(result.eos_cost, matrix.cost(rid, 0).unwrap());
    assert_eq!(result.cost, cost + u64::from(result.eos_cost));
    assert_eq!(
        result.value,
        result
            .tokens
            .iter()
            .map(|t| t.token.value.as_str())
            .collect::<String>()
    );
}

#[test]
fn global_path_beats_greedy_and_includes_both_boundaries() {
    let bytes = image(vec![
        token("あ", "安", 1, 1),
        token("あ", "亜", 5, 2),
        token("い", "胃", 1, 3),
        token("あい", "愛", 20, 1),
    ]);
    let dict = Dictionary::from_bytes(&bytes).unwrap();
    let matrix =
        Connection::from_costs(4, vec![0, 1, 2, 0, 50, 0, 0, 100, 0, 0, 0, 0, 3, 0, 0, 0]).unwrap();
    let result = Converter::new(&dict, &matrix, 3)
        .unwrap()
        .convert("あい")
        .unwrap();
    assert_eq!((result.value.as_str(), result.cost), ("亜胃", 11));
    check_result(&result, &matrix);
    for boundary in [(0, 1), (1, 0)] {
        let mut costs = vec![0; 16];
        costs[boundary.0 * 4 + boundary.1] = 100;
        let matrix = Connection::from_costs(4, costs).unwrap();
        let result = Converter::new(&dict, &matrix, 3)
            .unwrap()
            .convert("あ")
            .unwrap();
        assert_eq!(result.value, "亜");
        check_result(&result, &matrix);
    }
}

#[test]
fn ties_use_stable_insertion_order_including_eos() {
    let bytes = image(vec![
        token("あ", "甲", 5, 1),
        token("あ", "乙", 5, 1),
        token("い", "胃", 5, 1),
        token("あい", "一語", 10, 1),
    ]);
    let dict = Dictionary::from_bytes(&bytes).unwrap();
    let matrix = Connection::from_costs(3, vec![0; 9]).unwrap();
    let conv = Converter::new(&dict, &matrix, 2).unwrap();
    let first = dict.lookup_exact("あ").unwrap()[0].value.clone();
    for _ in 0..5 {
        assert_eq!(conv.convert("あ").unwrap().value, first);
        assert_eq!(conv.convert("あい").unwrap().value, "一語");
    }
}

#[test]
fn empty_unknown_unicode_and_finite_invalid_cost_have_explicit_semantics() {
    let bytes = image(vec![token("あ", "亜", 100, 1)]);
    let dict = Dictionary::from_bytes(&bytes).unwrap();
    let matrix = Connection::from_costs(3, vec![0, 0, 0, 0, 0, 0, 0, 0, 30000]).unwrap();
    let conv = Converter::new(&dict, &matrix, 2).unwrap();
    for query in ["", "あ😀x。", "か\u{3099}", "😀😀"] {
        let result = conv.convert(query).unwrap();
        check_result(&result, &matrix);
        assert_eq!(result.value, query.replace('あ', "亜"));
        for t in result.tokens.iter().filter(|t| t.unknown) {
            assert_eq!(t.token.cost, UNKNOWN_WORD_COST);
        }
    }
    assert_eq!(conv.convert("😀😀").unwrap().cost, 32767 * 2 + 30000);
    assert!(conv.convert("a\0b").is_err());
    assert!(conv.convert(&"あ".repeat(MAX_INPUT_BYTES / 3 + 1)).is_err());
}

fn enumerate(
    dict: &Dictionary<'_>,
    matrix: &Connection,
    query: &str,
    rid: u16,
    total: u64,
    best: &mut u64,
) {
    if query.is_empty() {
        *best = (*best).min(total + u64::from(matrix.cost(rid, 0).unwrap()));
        return;
    }
    let mut candidates = dict.lookup_prefix(query).unwrap();
    let scalar = query.chars().next().unwrap().to_string();
    candidates.push(token(&scalar, &scalar, UNKNOWN_WORD_COST, 4));
    for t in candidates {
        enumerate(
            dict,
            matrix,
            &query[t.key.len()..],
            t.rid,
            total + u64::from(matrix.cost(rid, t.lid).unwrap()) + u64::from(t.cost),
            best,
        );
    }
}

#[test]
fn dynamic_program_matches_exhaustive_path_enumeration() {
    let bytes = image(vec![
        token("あ", "亜", 10, 1),
        token("あ", "阿", 3, 2),
        token("い", "胃", 6, 3),
        token("あい", "愛", 8, 1),
        token("いあ", "異阿", 2, 2),
    ]);
    let dict = Dictionary::from_bytes(&bytes).unwrap();
    let mut seed = 42u32;
    for _ in 0..25 {
        let mut costs = vec![0; 25];
        for c in &mut costs[1..] {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            *c = (seed % 31) as u16;
        }
        let matrix = Connection::from_costs(5, costs).unwrap();
        let conv = Converter::new(&dict, &matrix, 4).unwrap();
        for query in ["あいあい", "いああ", "あ😀い", ""] {
            let mut best = u64::MAX;
            enumerate(&dict, &matrix, query, 0, 0, &mut best);
            let result = conv.convert(query).unwrap();
            assert_eq!(result.cost, best);
            check_result(&result, &matrix);
        }
    }
}

#[test]
fn converter_rejects_incompatible_pos_even_for_unqueried_words() {
    let bytes = image(vec![token("あ", "亜", 1, 3)]);
    let dict = Dictionary::from_bytes(&bytes).unwrap();
    let matrix = Connection::from_costs(3, vec![0; 9]).unwrap();
    assert!(Converter::new(&dict, &matrix, 1).is_err());
    assert!(Converter::new(&dict, &matrix, 0).is_err());
    assert!(Converter::new(&dict, &matrix, 3).is_err());
}

#[test]
fn conversion_cli_streams_crlf_and_empty_and_rejects_bad_inputs() {
    let dir = std::env::temp_dir().join(format!("mozrs-convert-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let dictionary = dir.join("dictionary.data");
    let connection = dir.join("connection.data");
    fs::write(&dictionary, image(vec![token("あ", "亜", 100, 1)])).unwrap();
    let matrix = Connection::from_costs(3, vec![0; 9]).unwrap();
    fs::write(&connection, matrix.to_bytes().unwrap()).unwrap();
    let bin = env!("CARGO_BIN_EXE_mozrs-convert");
    let mut child = Command::new(bin)
        .arg(&dictionary)
        .arg(&connection)
        .arg("2")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all("あ😀\r\n\n".as_bytes())
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    let responses: Vec<Conversion> = String::from_utf8(result.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0].value, "亜😀");
    assert_eq!(responses[1].cost, 0);
    for id in ["0", "3", "invalid"] {
        assert!(
            !Command::new(bin)
                .arg(&dictionary)
                .arg(&connection)
                .args([id, "あ"])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    fs::write(&connection, b"invalid").unwrap();
    let bad = Command::new(bin)
        .arg(&dictionary)
        .arg(&connection)
        .args(["2", "あ"])
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(bad.stdout.is_empty());
    fs::remove_dir_all(dir).unwrap();
}
