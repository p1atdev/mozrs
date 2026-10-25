# 依存関係と残る移植範囲

固定版 `c4b2aef7c65e61be394bacadaec200eb5b97f91c` に対する辞書生成の移植後の構成。

| 操作 | 必要なもの | Mozc C++ / Bazel |
|---|---|---|
| ライブラリ・検索 CLI・生成 CLI のビルドと通常テスト | Cargo、Rust、対象環境のリンカー・システムライブラリ | 不要 |
| バイナリ辞書の検索 | Rust 製 `mozrs` と `dictionary.data` | 不要 |
| 整形済み JSONL / TSV からの辞書生成 | Rust 製 `mozrs-build` | 不要 |
| OSS 原データからの通常生成 | Rust 製 builder、Python 3.12+、固定版のデータ・Python スクリプト | 不要 |
| 独立した生成物・検索結果の比較 | 上記に加えて C++20 コンパイラー、Bazelisk、上流依存群 | 任意の検証時のみ |

`Cargo.toml` / `Cargo.lock` にある依存は serde / serde_json とその Rust 依存のみ。
この crate に `build.rs`・FFI・C/C++ ソースのコンパイルはない。`unsafe` は禁止。
依存 crate のビルドスクリプトは Rust の機能検出に使われ、native ライブラリを生成しない。
今回の移植で crate を追加していない。

Linux GNU 環境では Rust 標準ライブラリやリンク処理がシステムの C ABI に依存する。
検証した両 CLI の ELF は `libc.so.6` と `libgcc_s.so.1` を必要とし、動的ローダーも使う。
`libstdc++` や Mozc C++ はリンクしていない。Rust のリンク時に `cc` ドライバーを使う
環境もあるため、「C ツールチェーンや libc も一切不要」という意味ではない。
Python 処理を CPython で実行すると、その処理系と標準ライブラリの native 実装も残る。

## Rust に移した部分

- `SystemDictionaryBuilder`、値・キーの符号化、LOUDS trie / token array の生成、
  頻出品詞表、候補の安定した並べ替え、上流と同じコスト圧縮。
- `TextDictionaryLoader` の TSV 解析・ラベル処理、258 規則の濁点正規化、読み補正。
- `id.def` と `special_pos.def` から ZIP_CODE / ENGLISH 用の 2 品詞 ID を解決。
  固定版の `Zipcode` / `IsolatedWord` はリテラルの品詞名なので、汎用正規表現 matcher は不要。

これにより、通常生成から `gen_system_dictionary_data_main`、品詞用の
`dataset_writer_main`、`user_pos_manager.data`、Protobuf / Abseil のビルドを外した。
外側の `mozc.data` を組み立てる C++ データセット生成器は今回の出力に不要。

## 残した Python と任意の C++

`tools/build_oss.py` は、ハッシュ検証済み上流ソースの次の処理を直接起動する。

- `dictionary.gen_zip_code_seed` / `zip_code_util`: 固定版の郵便番号 CSV の整形。
- `dictionary.gen_aux_dictionary`: 手動・補助語彙への品詞とコストの割り当て。
- `dictionary.gen_filtered_dictionary`: 正規表現による語彙の除外。

これらは Python 標準ライブラリだけで動き、pip パッケージは不要。
ソースアーカイブには C++ ファイルも含まれるが、通常生成でコンパイル・実行・リンクしない。
次に Python 依存も減らすなら、この 3 処理とダウンロード用スクリプトが対象になる。
特に CP932、Unicode 正規化、正規表現の互換性を維持する必要がある。

`tools/oracle.cc` と `tools/validate.py` は上流 C++ を独立した比較基準として使う。
通常テストは保存済みの小さな C++ 期待値を読むため、C++ 環境を要求しない。
任意検証を残せば、将来の形式変更を双方が同じように誤実装するリスクを減らせる。
削除しても生成・検索機能は動くが、新しい実装を独立に照合する経路を失う。

文節分割、接続コストによる探索、予測、学習、リライター、セッション管理、OS の IME
連携は未移植。これらは現在の CLI の依存ではなく、完全な Mozc 移植に向けた別の作業。
Linux x86_64 で確認し、他 OS や libc を使わないターゲットでの検証はしていない。
