import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--toolchain", default="coaptic-esp-1.97")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    manifest = root / "tools/esphome/rust/Cargo.toml"
    subprocess.run(["cargo", "+" + args.toolchain, "rustc", "-Zbuild-std=core", "--locked",
                    "--release", "--target", "xtensa-esp32s3-none-elf", "--no-default-features",
                    "--features", "network,standalone", "--manifest-path", str(manifest),
                    "--crate-type", "staticlib"], check=True, cwd=root)
    archive = manifest.parent / "target/xtensa-esp32s3-none-elf/release/libcoaptic_esphome_probe.a"
    destination = root / "tools/esphome/components/coaptic_network/lib"
    destination.mkdir(exist_ok=True)
    shutil.copyfile(archive, destination / archive.name)
    files = [root / "Cargo.toml", manifest, manifest.with_name("Cargo.lock"),
             root / "crates/qualification-no-std/Cargo.toml"]
    for source in [root / "src", manifest.parent / "src", root / "crates/qualification-no-std/src"]:
        files.extend(sorted(source.rglob("*.rs")))
    report = {"schema": "coaptic-esphome-archive/1", "target": "xtensa-esp32s3-none-elf",
              "features": ["network", "standalone"],
              "compiler": subprocess.check_output(["rustc", "+" + args.toolchain, "-Vv"], text=True),
              "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
              "sources": {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest() for p in files}}
    (destination / "build.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
