use mozrs::connection::{Connection, unknown_pos_id};
use std::{fs, io::Cursor, process::Command};

const IMAGE: &[u8] = include_bytes!("fixtures/connection.data");
const RAW: &str = include_str!("fixtures/connection.txt");
const IDS: &str = include_str!("fixtures/connection-id.def");
const SPECIAL: &str = include_str!("fixtures/connection-special-pos.def");

#[test]
fn rust_connection_generation_and_loading_match_upstream_python() {
    let matrix = Connection::from_text(Cursor::new(RAW), IDS, SPECIAL).unwrap();
    assert_eq!(matrix.size(), 11);
    assert_eq!(unknown_pos_id(IDS).unwrap(), 1);
    assert_eq!(matrix.to_bytes().unwrap(), IMAGE);
    let loaded = Connection::from_bytes(IMAGE).unwrap();
    let expected: Vec<_> = include_bytes!("fixtures/connection-costs.bin")
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(loaded.costs(), expected);
    for r in 0..11 {
        for l in 0..11 {
            assert_eq!(loaded.cost(r, l).unwrap(), matrix.cost(r, l).unwrap());
        }
    }
    assert_eq!(loaded.cost(0, 9).unwrap(), 0);
    assert_eq!(loaded.cost(9, 0).unwrap(), 0);
    assert_eq!(loaded.cost(9, 10).unwrap(), 30000);
    assert!(loaded.cost(11, 0).is_err());
    assert!(loaded.cost(0, 11).is_err());
}

#[test]
fn malformed_connection_images_never_panic_and_truncations_fail() {
    for end in 0..IMAGE.len() {
        assert!(Connection::from_bytes(&IMAGE[..end]).is_err());
    }
    for (offset, bytes) in [
        (0, [0, 0]),
        (2, [64, 0]),
        (4, [0, 0]),
        (4, [1, 32]),
        (6, [4, 0]),
        (32, [255, 255]),
        (34, [255, 255]),
    ] {
        let mut bad = IMAGE.to_vec();
        bad[offset..offset + 2].copy_from_slice(&bytes);
        assert!(Connection::from_bytes(&bad).is_err(), "offset {offset}");
    }
    let mut extra = IMAGE.to_vec();
    extra.extend([0; 4]);
    assert!(Connection::from_bytes(&extra).is_err());
    let mut seed = 7u32;
    for _ in 0..500 {
        let mut data = IMAGE.to_vec();
        for _ in 0..3 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            data[seed as usize % IMAGE.len()] ^= (seed >> 24) as u8;
        }
        if let Ok(conn) = Connection::from_bytes(&data) {
            assert_eq!(conn.costs().len(), conn.size() * conn.size());
        }
    }
}

#[test]
fn incomplete_out_of_range_and_incompatible_text_is_rejected() {
    for raw in ["", "2\n0\n", "1\n0\n1\n", "1\n-1\n", "1\n65536\n"] {
        assert!(Connection::from_text(Cursor::new(raw), "0 BOS", "").is_err());
    }
    for ids in [
        "",
        "1 BOS",
        "0 BOS\n2 noun",
        "0 BOS\n0 duplicate",
        "0 BOS extra",
    ] {
        assert!(Connection::from_text(Cursor::new("1\n0\n"), ids, "").is_err());
    }
    assert!(unknown_pos_id("0 BOS").is_err());
    assert!(Connection::from_costs(0, vec![]).is_err());
    assert!(Connection::from_costs(2, vec![0; 3]).is_err());
    assert!(Connection::from_costs(1, vec![1]).is_err());
    let zero = Connection::from_costs(1, vec![0]).unwrap();
    assert_eq!(
        Connection::from_bytes(&zero.to_bytes().unwrap())
            .unwrap()
            .cost(0, 0)
            .unwrap(),
        0
    );
}

#[test]
fn connection_builder_cli_and_failed_input_preserve_output() {
    let dir = std::env::temp_dir().join(format!("mozrs-connection-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let output = dir.join("connection.data");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let command = |raw: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_mozrs-build-connection"))
            .arg(raw)
            .arg(fixture.join("connection-id.def"))
            .arg(fixture.join("connection-special-pos.def"))
            .arg(&output)
            .output()
            .unwrap()
    };
    let result = command(&fixture.join("connection.txt"));
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(fs::read(&output).unwrap(), IMAGE);
    let bad = dir.join("bad.txt");
    fs::write(&bad, b"9\n\xff").unwrap();
    assert!(!command(&bad).status.success());
    assert_eq!(fs::read(&output).unwrap(), IMAGE);
    fs::remove_dir_all(dir).unwrap();
}
