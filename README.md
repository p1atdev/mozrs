# mozrs

Mozc の OSS データを使い、辞書の生成・検索と基本的な文章変換を Rust で行うライブラリと CLI です。
辞書ビルダー、完全一致・共通接頭辞検索、接続表生成、最小コスト経路の探索を実装しています。
対象は upstream commit
[`c4b2aef7c65e61be394bacadaec200eb5b97f91c`](https://github.com/google/mozc/commit/c4b2aef7c65e61be394bacadaec200eb5b97f91c)
から生成した **`dictionary.data` と非量子化の `connection.data`** を対象にします。
文節操作やリライターを備えた完全な Mozc 変換器ではありません。

## ビルドと試用

Rust 2024 edition 対応のツールチェーンを使用します。
通常の Rust ビルド・テストには Mozc C++、Bazel、実辞書は不要です。
OSS 実辞書の生成には Python 3.12+ も使います。依存関係の境界は
[docs/dependencies.md](docs/dependencies.md) に記載しています。

```sh
cargo build --locked --release
cargo test --locked
cargo run --locked -- tests/fixtures/synthetic.data exact あ
cargo run --locked -- tests/fixtures/synthetic.data prefix あいうえお
```

同梱辞書は互換性テスト用の小型の合成データです。
実際の語彙を使う場合は、次の生成コマンドで辞書を用意してください。

## OSS 実辞書を生成する（C++ / Bazel 不要）

```sh
python3 tools/build_oss.py
target/release/mozrs dictionary-build/dictionary.data check
target/release/mozrs dictionary-build/dictionary.data exact にほん
target/release/mozrs-convert dictionary-build/dictionary.data \
  dictionary-build/connection.data 1841 わたしはにほんごをべんきょうしています
```

初回は固定版の Mozc ソースと郵便番号データを取得し、SHA-256 を検証します。
既存の Python 処理で郵便番号・補助語彙・語彙フィルターを準備し、Rust で
品詞 ID の解決、濁点の正規化、読み補正、バイナリ辞書・接続表の生成を行います。
`user_pos_manager.data`、Protobuf、Abseil、C++ 辞書生成器は使いません。

出力は `dictionary-build/dictionary.data` と `connection.data`、入力のハッシュと件数は
`dictionary-build/manifest.json`、中間語彙は `prepared/`、データのライセンス資料は
`licenses/` に保存します。`--cache DIR`、`--output DIR` で変更できます。
`--builder /path/to/mozrs-build` を指定すると既存バイナリと同じディレクトリの
`mozrs-build-connection` を使い、Cargo を起動しません。接続表生成器は
`--connection-builder PATH` で別途指定できます。
通常の生成には比較用 C++ ツールのビルドも実行も不要です。

## 基本的な文章変換

```sh
mozrs-convert DICTIONARY.data CONNECTION.data UNKNOWN_POS_ID [QUERY ...]
```

読みから最小コストの候補を1つ返します。上記の実例では
「私は日本語を勉強しています」、総コスト `17015` を返します。
`QUERY` を省略すると UTF-8 の標準入力を一行ずつ読みます。出力は JSONL で、
`query`、変換文字列 `value`、総コスト `cost`、末尾の接続コスト `eos_cost`、
選択した単語列 `tokens` を含みます。

`UNKNOWN_POS_ID` は同じ品詞定義から求めた未知語用 ID です。この固定版 OSS は `1841`。
生成時の `manifest.json` の `connection.unknown_pos_id` に記録しています。
辞書と接続表は同じ品詞定義・上流版の組で使ってください。範囲外 ID は拒否しますが、
同じサイズで意味の違う品詞表との組み合わせまでは検出できません。

変換の規則は以下のとおりです。

- 各文字位置で既存の共通接頭辞検索を行い、単語候補をつないで最適経路を求めます。
- 総コストは **復号済みの単語コストの合計＋接続コストの合計**。接続は
  前の語の `rid` → 次の語の `lid`。文頭・文末の品詞 ID はともに `0` です。
- すべての Unicode 文字位置に、原文1スカラーを保つ未知語候補を追加します。
  単語コストは `32767`、左右品詞は指定 ID。辞書語があっても追加します。
  数字・英字・カタカナの連続部分を特別にまとめる処理はありません。
- `30000` の接続コストも有限のペナルティとして加算します。接続禁止にはしません。
- 同点では、開始 byte 位置の昇順、辞書の接頭辞・格納順、最後に未知語、という
  ノード追加順で先に見つかった経路を保持します。文末でも同じ規則です。
- 入力の正規化・ローマ字変換は行いません。空入力は空の候補・コスト `0`。
  JSON 内の `start` / `end` は元の入力の UTF-8 byte オフセットです。

`tokens` は単語の経路であり、編集可能な Mozc の文節ではありません。
各要素は辞書 token の各フィールドに `start`、`end`、`unknown`、
直前からの `transition_cost` を追加しています。
上限は入力 4096 byte、20万ノード、中間遷移比較1000万回です。
超過時はエラーにし、候補を間引いた近似結果は返しません。NUL も拒否します。

接続表を単独で生成する場合：

```sh
mozrs-build-connection connection_single_column.txt id.def special_pos.def connection.data
```

上流と同じ行ごとの最頻値・2段階ビットマップによる非量子化形式を生成します。
品詞定義は `0` 始まりの連番、コストは `0..65535`、元データは行優先の正方行列です。
特殊品詞の行・列は上流と同じ `30000`、文頭・文末との接続は `0` を補います。
量子化された resolution 64 等の接続表は未対応で、読み込み時に拒否します。
接続表は読み込み時に密行列へ展開します（OSS では約14.3 MBのコスト配列）。

## 整形済み語彙から生成する

```sh
target/release/mozrs-build --output /tmp/synthetic.data --format jsonl \
  --min-key-length-for-small-cost 1 tests/fixtures/synthetic-input.jsonl
target/release/mozrs /tmp/synthetic.data exact あ

target/release/mozrs-build --output /tmp/loader.data \
  --id-def tests/fixtures/loader-id.def \
  --special-pos tests/fixtures/loader-special-pos.def \
  --corrections tests/fixtures/loader-corrections.tsv tests/fixtures/loader.tsv
```

`--format jsonl` は各行を `Token` として受け取り、文字の正規化を行いません。
既定の `tsv` は `読み TAB 左品詞ID TAB 右品詞ID TAB コスト TAB 表記 [TAB ラベル]`
を読み、上流の濁点正規化を適用します。複数の入力ファイルを指定できます。
ZIP_CODE・ENGLISH ラベルを使う場合は `--id-def` と `--special-pos` が必要です。
SPELLING_CORRECTION ラベルは属性 1 になります。

`--corrections` は `表記 TAB 誤った読み TAB 正しい読み` の上流形式です。
同じ表記の最大コストに 2302 を加えて語を追加します。補正ファイル由来の属性は
上流どおり 0 で、ラベルによる属性 1 とは区別します。重複語と安定した入力順を保持します。

上流互換の既定値では、6 文字以上で条件を満たす読みのコストの下位 8 bit を捨てます。
合成 fixture は境界を検証するため閾値 1 です。この圧縮も含めて生成物を比較しています。
`--min-key-length-for-small-cost 257` で無効化できます（符号化後の読みは最大 256 byte）。
この設定変更時は上流の既定出力とのバイト一致を期待できません。

品詞 ID は 0〜4095、コストは 0〜32767、属性は 0 / 1 に限ります。
空の読み・表記、NUL、256 byte を超える符号化済みの読み・表記、不明なラベルや
不正な数値はエラーにします。CLI は生成物を全件検証してから出力ファイルを開きます。
出力先が既存ファイルの場合は上書きします。全語彙をメモリに載せる実装です。

## CLI

```sh
mozrs DICTIONARY.data exact QUERY [QUERY ...]
mozrs DICTIONARY.data prefix QUERY [QUERY ...]
mozrs DICTIONARY.data check
```

`QUERY` を省略すると、標準入力から UTF-8 の読みを一行ずつ受け取ります。
空行も空の検索として処理します。出力は一入力につき一行の JSON です。

```sh
printf 'あ\nあいうえお\n' | target/release/mozrs tests/fixtures/synthetic.data prefix
```

`exact あ` は読みが「あ」の語だけを返します。
`prefix あいうえお` は「あ」「あい」「あいう」など、入力の先頭に一致する
辞書の読みを返します。「あ」から「あいう」を補完する予測検索ではありません。

```json
{"query":"不存在","mode":"exact","tokens":[]}
```

各 token のフィールドは `key`（読み）、`value`（表記）、`cost`、`lid`、
`rid`（左右の品詞 ID）、`attributes`（0: 通常、1: SPELLING_CORRECTION）です。
重複する表記も品詞やコストが違えば保持します。順番は上流の検索走査順で、
変換エンジンによる候補順位ではありません。コストはバイナリの復号値です。

CLI は出力前に全キー・値・トークンを検証します。ファイルや入力に問題が
ある場合は標準エラーに理由を出し、終了コード 1 を返します。
`check` はキー数・トークン数・対応する上流 revision を返します。

## ライブラリ

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let image = std::fs::read("tests/fixtures/synthetic.data")?;
    let dict = mozrs::Dictionary::from_bytes(&image)?;
    dict.validate()?; // 全体を検証。通常の lookup は参照したトークンを検証する。
    let tokens = dict.lookup_prefix("あいうえお")?;
    println!("{tokens:?}");
    Ok(())
}
```

辞書データは呼び出し元が所有します。構造は読み込み時に検証し、トークンは
検索時または `validate()` で検証します。`unsafe` は禁止しています。
LOUDS の探索用インデックスをメモリ上に展開する実装なので、Mozc と同等の
メモリ使用量を目標にした実装ではありません。

生成 API は `builder::build(Vec<Token>, &BuildOptions)` で `Vec<u8>` を返します。
TSV 入力用には `input::parse_tsv_line`、`PosIds::from_definitions`、
`input::add_reading_corrections` を公開しています。
文章変換は `Converter::new(&dictionary, &connection, unknown_pos_id)?.convert(query)`。
`connection::Connection` と `converter::Converter` はいずれも Rust の実装です。

## 任意の上流 C++ 比較を再現する

Linux、Python 3.12+、Bazelisk、C++20 コンパイラー、Cargo を用意してください。
Bazelisk は上流指定の Bazel 9.0.2 を使います。初回はネットワーク接続が必要です。

```sh
python3 tools/validate.py --bazelisk /path/to/bazelisk
```

スクリプトは次を行います。

1. 固定版の上流アーカイブを取得し、SHA-256 と展開されたソースを検証する。
2. キャッシュ内の上流コピーに比較用 CLI のビルドルールだけを追加する。
3. 上流の `SystemDictionaryBuilder` と `SystemDictionary` を使って比較用
   ツール・テスト辞書・OSS 辞書を生成する。辞書の C++ 実装は変更しない。
4. 同梱合成データ・前処理の期待値を再生成し、Rust テストを実行する。
5. Rust で合成辞書と OSS 実辞書を生成し、C++ 生成物とのバイト一致を検証する。
   郵便番号・補助語彙・フィルター結果も上流 Bazel の中間ファイルと照合する。
6. 辞書を Rust で全件検証し、両実装の検索結果の全フィールド・件数・順序を比較する。
   実辞書の検索比較は固定 seed のサンプルを使う。
7. 接続表の生成バイト列と全セルの復号値、同じ基本ノードを使った上流 Viterbi の
   経路・コストを照合する。比較用に上流の OSS `mozc.data` も生成する。

変換比較は、上流の既存 test peer を通じて **未変更の `ImmutableConverter::Viterbi`**
を呼びます。ノードは今回の基本ポリシーで作り、1区間・履歴なし・接続ボーナスなし。
上流の `MakeLattice`、文節化、N-best、リライターを通した製品の変換結果との一致は
主張しません。変換比較の詳細と出力は `validation-output/conversion/` に保存します。

生成物とログは `validation-output/` に置きます。実辞書は
`validation-output/upstream/mozc-<revision>/src/bazel-bin/data_manager/oss/dictionary.data`
です。同じ処理内で Rust 生成物も `validation-output/rust-oss/dictionary.data` に保存します。
`--cache`・`--output` で保存先、`--sample` で各辞書の抽出読み数を変更できます。
環境変数 `CARGO_TARGET_DIR` にも対応しています。IME のインストールや OS 設定変更は行いません。

## 対応範囲と検証

対応するのは little-endian の OSS system dictionary 形式と、標準の
濁点・小書き文字を区別する検索です。外側の `mozc.data`、Google 日本語入力の
非公開データ、予測検索、文節分割、N-best、リライター、学習、OS 連携は対象外です。
異なる上流版やプラットフォームで生成した辞書の互換性は未検証です。

不正な長さ・インデックス・Unicode・トークン符号はエラーにします。
辞書コンテナ自体にチェックサムがないため、別の有効な語に変わるような
データ改変まで検出するものではありません。メモリ・速度の比較測定、網羅的な
fuzzing、全読みの C++ 比較はまだ行っていません。

実測した検証範囲は [docs/validation.md](docs/validation.md) に記載しています。
コードは BSD-3-Clause、移植元と辞書のライセンス境界は [NOTICE.md](NOTICE.md) を参照してください。
