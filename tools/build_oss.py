#!/usr/bin/env python3
"""Generate OSS dictionary.data using Rust and upstream Python (no C++/Bazel).

Requires Python 3.12+, Cargo, and network access on the first run. Inputs are
pinned and hash-checked. Real dictionary licenses remain separate from code.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import zipfile

from upstream import ARCHIVE_SHA256, REPO, REVISION, digest, download, prepare_source

POSTAL_REVISION = "621d059fbcbfae17bfca15b439692bae934268c3"
POSTAL = (
    ("ken_all", "KEN_ALL.CSV", "3adc97d8e19e1168a6cdcb2fce759d0e9e613a9a3b630cc180aecfd56ae81eaf"),
    ("jigyosyo", "JIGYOSYO.CSV", "14be1030f607c5f819f2f45a485e89511907bdd1d62e7fb1931b6e48a3431c3a"),
)


def run(command, cwd, log):
    print("Running: " + " ".join(map(str, command)), flush=True)
    with log.open("w") as stream:
        result = subprocess.run(command, cwd=cwd, stdout=stream, stderr=subprocess.STDOUT)
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}): {log}\n{log.read_text()[-6000:]}")


def build(cache, output, builder=None, connection_builder=None):
    output.mkdir(parents=True, exist_ok=True)
    prepared = output / "prepared"
    prepared.mkdir(exist_ok=True)
    src = prepare_source(cache)
    for name, member, sha in POSTAL:
        archive = cache / f"{name}.zip"
        download(f"https://github.com/hiroyuki-komatsu/japanpost_zipcode/raw/{POSTAL_REVISION}/{name}.zip", archive, sha)
        with zipfile.ZipFile(archive) as zip_file:
            (prepared / member).write_bytes(zip_file.read(member))
    postal = prepared / "zip_code.tsv"
    run([sys.executable, "-B", "-m", "dictionary.gen_zip_code_seed", "--output", postal,
         "--zip_code", prepared / "KEN_ALL.CSV", "--jigyosyo", prepared / "JIGYOSYO.CSV"],
        src, output / "postal.log")
    # $(locations ...) sorts Bazel's generated postal path before source paths.
    # Preserve that order, including stable ties in auxiliary word selection.
    base = [postal] + [src / f"data/dictionary_oss/dictionary{i:02}.txt" for i in range(10)]
    aux = prepared / "aux_dictionary.txt"
    filtered = prepared / "dictionary.txt"
    run([sys.executable, "-B", "-m", "dictionary.gen_aux_dictionary", "--strict", "--output", aux,
         "--aux_tsv", src / "data/dictionary_oss/aux_dictionary.tsv",
         "--id_def", src / "data/dictionary_oss/id.def", "--words_tsv",
         src / "data/dictionary_manual/places.tsv", src / "data/dictionary_manual/words.tsv",
         "--dictionary_txts", *base], src, output / "aux.log")
    run([sys.executable, "-B", "-m", "dictionary.gen_filtered_dictionary", "--output", filtered,
         "--filter_tsv", src / "data/dictionary_oss/dictionary_filter.tsv", "--dictionary_txts", *base],
        src, output / "filter.log")
    if builder is None:
        run(["cargo", "build", "--locked", "--release", "--bins"], REPO, output / "cargo.log")
        target = Path(os.environ.get("CARGO_TARGET_DIR", REPO / "target"))
        if not target.is_absolute():
            target = REPO / target
        builder = target / "release" / ("mozrs-build.exe" if os.name == "nt" else "mozrs-build")
    dictionary = output / "dictionary.data"
    inputs = [aux, filtered, src / "data/dictionary_manual/domain.txt"]
    run([builder, "--output", dictionary, "--id-def", src / "data/dictionary_oss/id.def",
         "--special-pos", src / "data/rules/special_pos.def",
         "--corrections", src / "data/dictionary_oss/reading_correction.tsv", *inputs],
        REPO, output / "builder.log")
    if connection_builder is None:
        connection_builder = builder.with_name("mozrs-build-connection.exe" if os.name == "nt" else "mozrs-build-connection")
    connection = output / "connection.data"
    run([connection_builder, src / "data/dictionary_oss/connection_single_column.txt",
         src / "data/dictionary_oss/id.def", src / "data/rules/special_pos.def", connection],
        REPO, output / "connection-builder.log")
    notices = output / "licenses"
    notices.mkdir(exist_ok=True)
    shutil.copyfile(src.parent / "LICENSE", notices / "Mozc-LICENSE.txt")
    shutil.copyfile(src / "data/dictionary_oss/README.txt", notices / "dictionary-README.txt")
    shutil.copyfile(REPO / "NOTICE.md", notices / "mozrs-NOTICE.md")
    report = {"upstream_revision": REVISION, "upstream_archive_sha256": ARCHIVE_SHA256,
              "postal_revision": POSTAL_REVISION, "postal_sha256": {name: sha for name, _, sha in POSTAL},
              "prepared_sha256": {p.name: digest(p) for p in [postal, *inputs]},
              "dictionary_sha256": digest(dictionary), "result": json.loads((output / "builder.log").read_text()),
              "connection_sha256": digest(connection), "connection": json.loads((output / "connection-builder.log").read_text()),
              "generation": "upstream Python preprocessing + Rust POS/TSV/corrections/binary builder; no Mozc C++ or Bazel"}
    (output / "manifest.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(f"Generated {dictionary}: {report['dictionary_sha256']}", flush=True)
    return dictionary, src


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", type=Path, default=REPO / "dictionary-build/cache")
    parser.add_argument("--output", type=Path, default=REPO / "dictionary-build")
    parser.add_argument("--builder", type=Path, help="use an already built mozrs-build and sibling mozrs-build-connection")
    parser.add_argument("--connection-builder", type=Path, help="override the mozrs-build-connection executable")
    args = parser.parse_args()
    build(args.cache.resolve(), args.output.resolve(), args.builder.resolve() if args.builder else None,
          args.connection_builder.resolve() if args.connection_builder else None)


if __name__ == "__main__":
    main()
