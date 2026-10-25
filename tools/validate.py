#!/usr/bin/env python3
"""Rebuild the pinned C++ oracle and compare Rust on identical dictionaries.

Writes only to the supplied cache/output directories. Requires Python 3.12+,
Bazelisk, a C++20 compiler and Cargo. No IME installation is performed.
"""
import argparse
import hashlib
import itertools
import json
import os
from pathlib import Path
import random
import shutil
import subprocess
import tarfile
import urllib.request

import build_oss

REVISION = "c4b2aef7c65e61be394bacadaec200eb5b97f91c"
ARCHIVE_SHA256 = "90023a57813911b87e758346d081d8a7f6101e5ca1ff553ca3f10cd27e1e1c8c"
REPO = Path(__file__).resolve().parents[1]


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def prepare(cache):
    archive = cache / "upstream.tar.gz"
    if not archive.exists():
        url = f"https://codeload.github.com/google/mozc/tar.gz/{REVISION}"
        print(f"Downloading pinned source: {url}", flush=True)
        with urllib.request.urlopen(url, timeout=60) as src, archive.open("wb") as dst:
            shutil.copyfileobj(src, dst)
    if digest(archive) != ARCHIVE_SHA256:
        raise RuntimeError(f"archive checksum mismatch: {archive}")
    root = cache / f"mozc-{REVISION}"
    with tarfile.open(archive) as tar:
        if not root.exists():
            tar.extractall(cache, filter="data")
        build_name = f"mozc-{REVISION}/src/dictionary/system/BUILD.bazel"
        original = tar.extractfile(build_name).read().decode()
        load, body = (REPO / "tools/oracle.BUILD.bazel").read_text().split("\n", 1)
        overlay = load + "\n" + original + "\n" + body
        # Verify the archived sources, not just the directory's name. The only
        # permitted change is the exact additive oracle BUILD overlay below.
        for member in tar:
            if not member.isfile():
                continue
            path = cache / member.name
            if member.name == build_name:
                if path.read_text() not in (original, overlay):
                    raise RuntimeError(f"unexpected upstream BUILD edits: {path}")
            elif path.read_bytes() != tar.extractfile(member).read():
                raise RuntimeError(f"upstream source differs from pinned archive: {path}")
    src = root / "src"
    (src / "dictionary/system/BUILD.bazel").write_text(overlay)
    shutil.copyfile(REPO / "tools/oracle.cc", src / "dictionary/system/mozrs_oracle.cc")
    return src


def run_logged(command, cwd, log):
    with log.open("w") as stream:
        result = subprocess.run(command, cwd=cwd, stdout=stream, stderr=subprocess.STDOUT)
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}); see {log}\n{log.read_text()[-6000:]}")


def compare_file(expected, actual):
    count = tokens = 0
    with expected.open() as left, actual.open() as right:
        for count, (a, b) in enumerate(itertools.zip_longest(left, right), 1):
            if a is None or b is None or json.loads(a) != json.loads(b):
                raise RuntimeError(f"lookup mismatch at line {count}: {expected} vs {actual}\nC++: {a}\nRust: {b}")
            tokens += len(json.loads(a)["tokens"])
    return {"queries": count, "returned_tokens": tokens, "mismatches": 0}


def sample_queries(files, limit):
    keys = set()
    for path in files:
        with path.open() as stream:
            for line in stream:
                if line.strip() and not line.startswith("#"):
                    keys.add(line.split("\t", 1)[0].rstrip("\n"))
    keys = sorted(keys)
    selected = sorted(random.Random(20261025).sample(keys, min(limit, len(keys))))
    return selected + ["", "不存在😀mozrs", "あいうえお", "う゛", "ゔ", "ゕゖ", "ABC-123"]


def compare_corpus(name, dictionary, files, oracle, rust, output, limit):
    queries = sample_queries(files, limit)
    counts = json.loads(subprocess.check_output([rust, dictionary, "check"], text=True))
    result = {"dictionary_sha256": digest(dictionary), "validated": counts}
    for mode in ("exact", "prefix"):
        # Prefix queries intentionally extend the reading, distinguishing
        # common-prefix lookup from exact lookup and predictive completion.
        inputs = [q + "です" if mode == "prefix" and q else q for q in queries]
        payload = "\n".join(inputs) + "\n"
        query_file = output / f"{name}-{mode}-queries.txt"
        query_file.write_text(payload)
        cpp_path = output / f"{name}-{mode}-cpp.jsonl"
        rust_path = output / f"{name}-{mode}-rust.jsonl"
        for command, path in [([oracle, dictionary, mode], cpp_path), ([rust, dictionary, mode], rust_path)]:
            with path.open("w") as stream:
                subprocess.run(command, input=payload, text=True, stdout=stream, check=True)
        result[mode] = compare_file(cpp_path, rust_path)
    print(f"{name}: {json.dumps(result, ensure_ascii=False)}", flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", type=Path, default=REPO / "validation-output/upstream")
    parser.add_argument("--output", type=Path, default=REPO / "validation-output")
    parser.add_argument("--bazelisk", default="bazelisk")
    parser.add_argument("--sample", type=int, default=2000)
    parser.add_argument("--jobs", type=int, default=4)
    args = parser.parse_args()
    if args.sample < 1 or args.jobs < 1:
        parser.error("--sample and --jobs must be positive")
    cache, output = args.cache.resolve(), args.output.resolve()
    cache.mkdir(parents=True, exist_ok=True)
    output.mkdir(parents=True, exist_ok=True)
    print("Checking pinned upstream source...", flush=True)
    src = prepare(cache)
    print("Building C++ oracle and both upstream dictionaries...", flush=True)
    run_logged([
        args.bazelisk, f"--output_user_root={cache / 'bazel-cache'}", "build",
        "//dictionary/system:mozrs_oracle",
        "//data_manager/testing:mozc_dataset_for_testing@dictionary",
        "//data_manager/oss:mozc_dataset_for_oss@dictionary",
        "--config=oss_linux", f"--jobs={args.jobs}",
    ], src, output / "bazel-build.log")
    print("Building Rust...", flush=True)
    run_logged(["cargo", "build", "--locked", "--release"], REPO, output / "cargo-build.log")
    target = Path(os.environ.get("CARGO_TARGET_DIR", REPO / "target"))
    if not target.is_absolute():
        target = REPO / target
    rust = target / "release/mozrs"
    oracle = src / "bazel-bin/dictionary/system/mozrs_oracle"
    generated = output / "synthetic.data"
    truth = output / "synthetic.jsonl"
    prepared_fixture = output / "synthetic-input.jsonl"
    with truth.open("w") as stream:
        subprocess.run([oracle, "fixture", generated], stdout=stream, check=True)
    with prepared_fixture.open("w") as stream:
        subprocess.run([oracle, "fixture-inputs", "unused"], stdout=stream, check=True)
    fixtures = REPO / "tests/fixtures"
    loader_dictionary = output / "loader.data"
    loader_inputs = output / "loader-input.jsonl"
    loader_args = [loader_dictionary, "10", "11", fixtures / "loader-corrections.tsv", fixtures / "loader.tsv"]
    run_logged([oracle, "build-tsv", *loader_args], REPO, output / "loader-cpp-build.log")
    with loader_inputs.open("w") as stream:
        subprocess.run([oracle, "tsv-inputs", *loader_args], stdout=stream, check=True)
    for path in (generated, truth, prepared_fixture, loader_dictionary, loader_inputs):
        checked_in = REPO / "tests/fixtures" / path.name
        if path.read_bytes() != checked_in.read_bytes():
            raise RuntimeError(f"regenerated fixture differs: {path} vs {checked_in}")
    builder = target / "release/mozrs-build"
    rust_synthetic = output / "rust-synthetic.data"
    run_logged([builder, "--format", "jsonl", "--min-key-length-for-small-cost", "1", "--output",
                rust_synthetic, prepared_fixture], REPO, output / "synthetic-rust-build.log")
    rust_loader = output / "rust-loader.data"
    run_logged([builder, "--output", rust_loader, "--id-def", fixtures / "loader-id.def",
                "--special-pos", fixtures / "loader-special-pos.def", "--corrections",
                fixtures / "loader-corrections.tsv", fixtures / "loader.tsv"], REPO, output / "loader-rust-build.log")
    for cpp, rust_image in [(generated, rust_synthetic), (loader_dictionary, rust_loader)]:
        if cpp.read_bytes() != rust_image.read_bytes():
            raise RuntimeError(f"C++/Rust generated bytes differ: {cpp} vs {rust_image}")
    if (REPO / "src/normalize-voiced-sound.tsv").read_bytes() != (src / "data/preedit/normalize-voiced-sound.tsv").read_bytes():
        raise RuntimeError("normalization table differs from pinned upstream")
    run_logged(["cargo", "test", "--locked"], REPO, output / "cargo-test.log")
    report = {
        "upstream_revision": REVISION, "upstream_archive_sha256": ARCHIVE_SHA256,
        "sample_seed": 20261025, "sample_limit": args.sample,
        "synthetic": {"dictionary_sha256": digest(generated), "expected_sha256": digest(truth),
                      "cases": len(truth.read_text().splitlines()), "regeneration_matches": True},
    }
    report["testing"] = compare_corpus("testing", src / "bazel-bin/data_manager/testing/dictionary.data",
        [src / "data/test/dictionary/dictionary.txt"], oracle, rust, output, args.sample)
    report["oss"] = compare_corpus("oss", src / "bazel-bin/data_manager/oss/dictionary.data",
        sorted((src / "data/dictionary_oss").glob("dictionary0[0-9].txt")), oracle, rust, output, args.sample)
    # This calls the standalone normal generation pipeline. The C++ oracle is
    # used only after generation, as an independent compatibility check.
    rust_oss, normal_src = build_oss.build(cache, output / "rust-oss", builder)
    cpp_oss = src / "bazel-bin/data_manager/oss/dictionary.data"
    if rust_oss.read_bytes() != cpp_oss.read_bytes():
        raise RuntimeError("Rust-generated OSS dictionary differs from pinned C++ bytes")
    for name, upstream_path in [
        ("zip_code.tsv", "dictionary/zip_code.tsv"),
        ("aux_dictionary.txt", "data/dictionary_oss/aux_dictionary.txt"),
        ("dictionary.txt", "data/dictionary_oss/dictionary.txt"),
    ]:
        if (rust_oss.parent / "prepared" / name).read_bytes() != (src / "bazel-bin" / upstream_path).read_bytes():
            raise RuntimeError(f"Python preparation differs from Bazel: {name}")
    report["rust_builder"] = {"synthetic_bytes_identical": True, "loader_bytes_identical": True,
                              "oss_bytes_identical": True, "prepared_files_identical": True,
                              "loader_sha256": digest(rust_loader)}
    loader_queries = output / "loader-keys.txt"
    loader_queries.write_text("\n".join(json.loads(line)["key"] for line in loader_inputs.read_text().splitlines()) + "\n")
    report["rust_loader"] = compare_corpus("rust-loader", rust_loader, [loader_queries],
        oracle, rust, output, args.sample)
    report["rust_oss"] = compare_corpus("rust-oss", rust_oss,
        [rust_oss.parent / "prepared/aux_dictionary.txt", rust_oss.parent / "prepared/dictionary.txt",
         normal_src / "data/dictionary_manual/domain.txt"], oracle, rust, output, args.sample)
    report["limits"] = "Generated synthetic, loader and full OSS images are byte-identical to C++. All records decoded; lookup parity checked on recorded queries, not exhaustively on all real keys. No sentence conversion, IME or performance validation."
    (output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(f"PASS: {output / 'report.json'}", flush=True)


if __name__ == "__main__":
    main()
