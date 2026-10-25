"""Pinned data/source acquisition, independent of the optional C++ oracle."""
import hashlib
from pathlib import Path
import shutil
import tarfile
import urllib.request

REVISION = "c4b2aef7c65e61be394bacadaec200eb5b97f91c"
ARCHIVE_SHA256 = "90023a57813911b87e758346d081d8a7f6101e5ca1ff553ca3f10cd27e1e1c8c"
REPO = Path(__file__).resolve().parents[1]


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def download(url, path, sha256):
    if not path.exists():
        temporary = path.with_suffix(path.suffix + ".part")
        print(f"Downloading {url}", flush=True)
        with urllib.request.urlopen(url, timeout=60) as src, temporary.open("wb") as dst:
            shutil.copyfileobj(src, dst)
        if digest(temporary) != sha256:
            raise RuntimeError(f"download checksum mismatch: {temporary}")
        temporary.replace(path)
    if digest(path) != sha256:
        raise RuntimeError(f"cache checksum mismatch: {path}")


def prepare_source(cache):
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / "upstream.tar.gz"
    download(f"https://codeload.github.com/google/mozc/tar.gz/{REVISION}", archive, ARCHIVE_SHA256)
    # Separate from the optional Bazel checkout: no BUILD overlay or generated
    # C++/protobuf files are needed or read by the normal generation pipeline.
    destination = cache / "rust-source"
    root = destination / f"mozc-{REVISION}"
    with tarfile.open(archive) as tar:
        if not root.exists():
            tar.extractall(destination, filter="data")
        for member in tar:
            if member.isfile() and (destination / member.name).read_bytes() != tar.extractfile(member).read():
                raise RuntimeError(f"upstream source differs from pinned archive: {member.name}")
    return root / "src"
