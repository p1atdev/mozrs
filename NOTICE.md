# Attribution and data licensing

The binary format, codec transformations, lookup, builder and input behavior in
`src/{reader,trie,codec,lib,builder,input}.rs` are ported/adapted from Google Mozc at
`c4b2aef7c65e61be394bacadaec200eb5b97f91c` (BSD 3-Clause):

- `src/base/hash.cc`
- `src/dictionary/file/codec.{h,cc}`
- `src/dictionary/system/{codec,system_dictionary}.{h,cc}`
- `src/dictionary/system/token_decode_iterator.h`
- `src/dictionary/system/system_dictionary_builder.{h,cc}`
- `src/dictionary/text_dictionary_loader.{h,cc}`
- `src/dictionary/pos_util.py`, `src/data/rules/{pos_matcher_rule,special_pos}.def`
- `src/storage/louds/{louds,louds_trie,bit_vector_based_array}.{h,cc}`
- `src/storage/louds/{louds_trie_builder,bit_vector_based_array_builder,bit_stream}.{h,cc}`
- `src/data/preedit/hiragana-katakana.tsv`
- `src/data/preedit/normalize-voiced-sound.tsv` (copied verbatim into `src/`)

The original copyright and license text is retained in
`licenses/Mozc-BSD.txt`; `LICENSE` also licenses the new code under BSD-3-Clause.
This project is not an official Google product and has no Google endorsement.

`tests/fixtures/synthetic.data` and `synthetic.jsonl` are generated from the
handwritten synthetic token set in `tools/oracle.cc` using the pinned upstream
builder and lookup implementation. `synthetic-input.jsonl` stores those original
uncompressed tokens. The `loader*` fixtures use handwritten TSV, POS definitions
and reading corrections, plus all 258 BSD-licensed normalization rules above;
`loader-input.jsonl` and `loader.data` are produced by the C++ loader/builder.
They contain no copied IPAdic/OSS dictionary
corpus. They are provided under this project's BSD-3-Clause license.

The actual upstream OSS and test dictionaries are **not bundled**. The optional
generation and optional validation workflows download them into their local caches. Their licensing is
separate from the Rust code: upstream `LICENSE` and
`src/data/dictionary_oss/README.txt` contain the IPAdic/NAIST/ICOT conditions and
Okinawa Dictionary notice; upstream explicitly applies those conditions to
`src/data/test/dictionary` as well. Other inputs, such as generated postal-code
data, keep their respective upstream notices. Retain the applicable notices
when distributing generated real dictionaries; this project's BSD license
does not relicense them. The normal build copies upstream LICENSE and dictionary
README into its output's `licenses/` directory and records source hashes in
`manifest.json`. This does not replace the notices attached to other source data.

`tools/build_oss.py` invokes the pinned upstream Python postal, auxiliary-word and
filter generators without modifications; they retain their original headers.
Postal archives are pinned to japanpost_zipcode revision
`621d059fbcbfae17bfca15b439692bae934268c3`, matching Mozc's MODULE.bazel hashes.

Cargo dependencies retain their own licenses. Their exact versions and
checksums are recorded in `Cargo.lock`; consult each crate's distributed
license files when distributing a linked binary.
