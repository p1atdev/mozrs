"""Optional matched-Viterbi verification, called by tools/validate.py.

Uses upstream's unchanged private Viterbi via its existing test-peer friendship.
Node construction is intentionally restricted to the basic Rust policy. This
does not compare against full Mozc MakeLattice, segmentation, N-best or rewriters.
"""
import itertools
import json
import random
import struct
import subprocess
import sys

from upstream import REPO, digest


def logged(command, output, payload=None):
    with output.open("wb") as stream, output.with_suffix(output.suffix + ".log").open("wb") as errors:
        subprocess.run(command, input=payload, stdout=stream, stderr=errors, check=True)


def compare_conversion(name, queries, dictionary, connection, unknown, oracle, dataset, rust, output):
    payload = ("\n".join(queries) + "\n").encode()
    (output / f"{name}-queries.txt").write_bytes(payload)
    cpp = output / f"{name}-cpp.jsonl"
    actual = output / f"{name}-rust.jsonl"
    logged([oracle, "convert", dataset, dictionary, connection, str(unknown)], cpp, payload)
    logged([rust, dictionary, connection, str(unknown)], actual, payload)
    total = words = fallback = 0
    with cpp.open() as left, actual.open() as right:
        for total, (a, b) in enumerate(itertools.zip_longest(left, right), 1):
            if a is None or b is None or json.loads(a) != json.loads(b):
                raise RuntimeError(f"Viterbi mismatch {name}:{total}\nC++: {a}\nRust: {b}")
            result = json.loads(b)
            words += len(result["tokens"])
            fallback += sum(t["unknown"] for t in result["tokens"])
    return {"queries": total, "path_tokens": words, "unknown_tokens": fallback, "mismatches": 0}


def check(src, output, target, limit):
    output = output / "conversion"
    output.mkdir(exist_ok=True)
    fixtures = REPO / "tests/fixtures"
    oracle = src / "bazel-bin/converter/mozrs_converter_oracle"
    dataset = src / "bazel-bin/data_manager/oss/mozc.data"
    dictionary = output.parent / "rust-oss/dictionary.data"
    connection = output.parent / "rust-oss/connection.data"
    cpp_connection = src / "bazel-bin/data_manager/oss/connection.data"
    if connection.read_bytes() != cpp_connection.read_bytes():
        raise RuntimeError("OSS connection image differs from upstream Python generator")

    # Regenerate the small connection golden values from pinned Python, not Rust.
    sys.path.insert(0, str(src))
    from data_manager import gen_connection_data as generator
    matrix = generator.ParseConnectionFile(str(fixtures / "connection.txt"), 9, 2)
    raw = b"".join(struct.pack("<H", c) for row in matrix for c in row)
    modes = generator.CreateModeValueList(matrix)
    generator.CompressMatrixByModeValue(matrix, modes)
    binary = generator.BuildBinaryData(matrix, modes, False)
    if binary != (fixtures / "connection.data").read_bytes() or raw != (fixtures / "connection-costs.bin").read_bytes():
        raise RuntimeError("connection golden values differ from pinned Python")
    connection_counts = {}
    for name, image in [("synthetic", fixtures / "connection.data"), ("oss", connection)]:
        cpp_dump = output / f"{name}-cpp-costs.bin"
        rust_dump = output / f"{name}-rust-costs.bin"
        logged([oracle, "connection", image], cpp_dump)
        logged([target / "release/mozrs-build-connection", "--dump-costs", image], rust_dump)
        if cpp_dump.read_bytes() != rust_dump.read_bytes():
            raise RuntimeError(f"decoded connection costs differ: {name}")
        connection_counts[name] = {"cells": cpp_dump.stat().st_size // 2, "mismatches": 0,
                                  "costs_sha256": digest(cpp_dump)}

    # Synthetic dictionary, real connection matrix: exercises independently built
    # token order, duplicate costs, fallback, normalization and correction flags.
    synthetic = output / "synthetic.data"
    logged([src / "bazel-bin/dictionary/system/mozrs_oracle", "build-tsv", synthetic,
            "10", "11", "-", fixtures / "conversion.tsv"], output / "synthetic-build.txt")
    rust_synthetic = output / "rust-synthetic.data"
    logged([target / "release/mozrs-build", "--output", rust_synthetic, fixtures / "conversion.tsv"], output / "synthetic-rust-build.txt")
    if synthetic.read_bytes() != rust_synthetic.read_bytes():
        raise RuntimeError("synthetic conversion dictionary generation differs")
    queries = ["", "あ", "あい", "いあ", "あいあい", "かな", "ゔ", "う゛", "あぼがど", "あ😀い", "x y", "😀😀", "。", "か\u3099"]
    synthetic_result = compare_conversion("synthetic", queries, rust_synthetic, connection, 1841,
        oracle, dataset, target / "release/mozrs-convert", output)

    # Reuse real regression readings only. Their full-engine expected outputs
    # are deliberately not treated as this basic converter's expected outputs.
    keys = set()
    corpus = [src / "data/test/quality_regression_test/oss.tsv", src / "data/test/quality_regression_test/regression.tsv"]
    for path in corpus:
        for line in path.read_text().splitlines():
            if not line or line.startswith("#"):
                continue
            columns = line.split("\t")
            if len(columns) >= 4 and columns[3].startswith("Conversion"):
                keys.add(columns[1])
    keys = sorted(keys)
    selected = sorted(random.Random(20261025).sample(keys, min(limit, len(keys))))
    extras = ["", "わたしはにほんごをべんきょうしています", "きょうはいいてんきです", "あ😀い", "😀😀",
              "XYZ-123", "１２３", "。！？", "か\u3099", "う゛", "ゔ", "a b", "\"\\"]
    result = compare_conversion("oss", selected + extras, dictionary, connection, 1841,
        oracle, dataset, target / "release/mozrs-convert", output)
    report = {"connection_sha256": digest(connection), "connection_bytes_identical": True,
              "connection_costs": connection_counts, "synthetic": synthetic_result, "oss": result,
              "corpus_readings": len(keys), "sample_seed": 20261025, "sample_limit": limit,
              "reference": "pinned ImmutableConverter::Viterbi with identical basic nodes, one unconstrained segment, no transition bonus",
              "limits": "Not full Mozc conversion quality or segmentation/N-best/rewriter/prediction parity. No performance benchmark."}
    (output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print("Conversion: " + json.dumps(report, ensure_ascii=False), flush=True)
    return report
