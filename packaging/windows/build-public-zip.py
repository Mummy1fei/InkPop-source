"""Package built InkPop executables with only hash-pinned public runtime resources."""
import argparse
import hashlib
import json
from pathlib import Path
from tempfile import NamedTemporaryFile
from zipfile import ZIP_DEFLATED, ZipFile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def package(release_dir, runtime_dir, output):
    if output.exists():
        raise FileExistsError(output)
    manifest = json.loads((HERE / "public-runtime.json").read_text(encoding="utf-8"))
    members = {}
    for name, expected in manifest["runtime"].items():
        resource = runtime_dir / name
        if not resource.resolve().is_relative_to(runtime_dir.resolve()) or digest(resource) != expected:
            raise ValueError(f"Unrecognized runtime resource: {name}")
        members[name] = resource
    members.update({name: ROOT / name for name in manifest["notices"]})
    members.update({"InkPop.exe": release_dir / "photocraft.exe", "InkPop-cli.exe": release_dir / "photocraft-cli.exe",
                    "README.txt": HERE / "README-public.txt"})
    for path in members.values():
        if not path.is_file():
            raise FileNotFoundError(path)
    output.parent.mkdir(parents=True, exist_ok=True)
    with NamedTemporaryFile(dir=output.parent, suffix=".partial", delete=False) as stream:
        temporary = Path(stream.name)
    try:
        with ZipFile(temporary, "w", compression=ZIP_DEFLATED, compresslevel=6) as archive:
            for name, path in sorted(members.items()):
                archive.write(path, name)
        with ZipFile(temporary) as archive:
            if sorted(archive.namelist()) != sorted(members):
                raise ValueError("ZIP members differ from the public inventory")
            for name, path in members.items():
                with archive.open(name) as stream:
                    if hashlib.file_digest(stream, "sha256").hexdigest() != digest(path):
                        raise ValueError(f"ZIP verification failed: {name}")
        # Windows rename fails if another process already published to this path.
        temporary.rename(output)
    finally:
        temporary.unlink(missing_ok=True)


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--release-dir", type=Path, required=True, help="Cargo release directory")
    cli.add_argument("--runtime-dir", type=Path, required=True, help="Extracted compatible InkPop runtime")
    cli.add_argument("--output", type=Path, required=True, help="New Windows release ZIP")
    args = cli.parse_args()
    package(args.release_dir, args.runtime_dir, args.output)
