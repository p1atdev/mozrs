use mozrs::Dictionary;

const IMAGE: &[u8] = include_bytes!("fixtures/synthetic.data");

fn check(data: &[u8]) -> mozrs::Result<usize> {
    let dict = Dictionary::from_bytes(data)?;
    dict.validate()
}

#[test]
fn every_truncation_of_a_real_cpp_dictionary_is_rejected() {
    for end in 0..IMAGE.len() {
        assert!(check(&IMAGE[..end]).is_err(), "truncation at {end}");
    }
    let mut extra = IMAGE.to_vec();
    extra.push(0);
    assert!(check(&extra).is_err());
}

#[test]
fn malformed_section_headers_are_rejected() {
    for (offset, bytes) in [
        (0, 0u32.to_le_bytes()),
        (4, 0u32.to_le_bytes()),
        (8, u32::MAX.to_le_bytes()),
        (8, 0u32.to_le_bytes()),
        (12, 0u32.to_le_bytes()),
    ] {
        let mut data = IMAGE.to_vec();
        data[offset..offset + 4].copy_from_slice(&bytes);
        assert!(check(&data).is_err(), "corrupt header at {offset}");
    }
}

#[test]
fn malformed_nested_indices_and_first_token_are_rejected() {
    let word =
        |offset: usize| u32::from_le_bytes(IMAGE[offset..offset + 4].try_into().unwrap()) as usize;
    // Pinned upstream writes value, token, key, POS sections in this order.
    let mut start = 8;
    let mut sections = Vec::new();
    for _ in 0..4 {
        let len = word(start);
        sections.push((start + 12, len));
        start += 12 + len.div_ceil(4) * 4;
    }
    let (value, _) = sections[0];
    let (tokens, token_len) = sections[1];
    let (key, _) = sections[2];
    for (offset, replacement) in [
        (value + 8, 16u32.to_le_bytes().to_vec()),
        (key + 4, u32::MAX.to_le_bytes().to_vec()),
        (value + 16, vec![0]),
        (tokens + 8, 0u32.to_le_bytes().to_vec()),
        (tokens + 12, 1u32.to_le_bytes().to_vec()),
        (tokens + token_len - 4, vec![0]),
        (tokens + 16 + word(tokens), vec![0x0c]),
    ] {
        let mut data = IMAGE.to_vec();
        data[offset..offset + replacement.len()].copy_from_slice(&replacement);
        assert!(check(&data).is_err(), "nested corruption at {offset}");
    }
}

#[test]
fn deterministic_mutations_never_panic() {
    // Some byte mutations are other valid dictionaries: require checked results,
    // not universal rejection. This is a bounded regression, not a fuzzing proof.
    let mut rng = 0x694d_62c8_59fa_51e3u64;
    for _ in 0..2000 {
        let mut data = IMAGE.to_vec();
        for _ in 0..3 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let offset = rng as usize % data.len();
            data[offset] ^= (rng >> 32) as u8;
        }
        if let Ok(dict) = Dictionary::from_bytes(&data) {
            let _ = dict.validate();
            let _ = dict.lookup_exact("あ");
            let _ = dict.lookup_prefix("😀あいう");
        }
    }
}
