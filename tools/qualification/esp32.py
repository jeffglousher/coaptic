import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
TOOLCHAIN = "1.97.1"
TARGET = "riscv32imc-unknown-none-elf"
MANIFEST = ROOT / "tools" / "qualification" / "esp32" / "Cargo.toml"


def read_capture(raw, case):
    lines = [line.split("COAPTIC_DEVICE ", 1)[1] for line in raw.splitlines() if "COAPTIC_DEVICE " in line]
    if len(lines) != 1 or "COAPTIC_DEVICE_FAIL" in raw:
        raise ValueError("missing, repeated or failed device result")
    result = json.loads(lines[0])
    if result.get("schema") != "coaptic-device/1" or result.get("chip") != "esp32c3":
        raise ValueError("wrong device/schema")
    if result.get("run_id") != case["run_id"] or result.get("oscore") is not case["oscore"]:
        raise ValueError("capture is not bound to this firmware/configuration")
    high = result.get("stack_high_water_bytes")
    capacity = result.get("stack_capacity_bytes")
    if result.get("passed") is not True or type(high) is not int or type(capacity) is not int or not 0 < high < capacity:
        raise ValueError("invalid runtime or stack result")
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--record", action="store_true")
    parser.add_argument("--capture-core", type=Path)
    parser.add_argument("--capture-oscore", type=Path)
    args = parser.parse_args()
    args.output = args.output.resolve()
    os.chdir(ROOT)
    if args.record:
        report = json.loads(args.output.read_text(encoding="utf-8"))
        if report.get("schema") != "coaptic-esp32-qualification/1" or len(report.get("cases", [])) != 2:
            parser.error("invalid build report")
        for case, capture in zip(report["cases"], [args.capture_core, args.capture_oscore], strict=True):
            case["runtime_passed"] = False
            if capture is None:
                case["runtime_error"] = "device capture required"
                continue
            try:
                firmware = Path(case["firmware"])
                if hashlib.sha256(firmware.read_bytes()).hexdigest() != case["firmware_sha256"]:
                    raise ValueError("firmware changed after build")
                raw = capture.read_text(encoding="utf-8")
                case["capture"] = str(capture.resolve())
                case["capture_sha256"] = hashlib.sha256(raw.encode()).hexdigest()
                case["runtime"] = read_capture(raw, case)
                case["runtime_passed"] = True
                case.pop("runtime_error", None)
            except (OSError, ValueError, KeyError) as error:
                case["runtime_error"] = str(error)
        report["runtime_passed"] = all(case["runtime_passed"] for case in report["cases"])
    else:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        report = {"schema": "coaptic-esp32-qualification/1", "runtime_passed": False,
                  "source": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
                  "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], text=True).strip()),
                  "compiler": subprocess.check_output(["rustc", "+" + TOOLCHAIN, "--version", "--verbose"], text=True),
                  "scope": "ESP32-C3 no-allocator App loopback, OSCORE authentication/replay, CPU0 stack after HAL initialization",
                  "unqualified": ["device execution until both captures are recorded", "radio/socket transport", "entropy integration", "allocator", "flash power loss", "other chips"],
                  "cases": []}
        for secured in [False, True]:
            run_id = secrets.token_hex(16)
            command = ["cargo", "+" + TOOLCHAIN, "build", "--locked", "--manifest-path", str(MANIFEST), "--release", "--target", TARGET]
            if secured:
                command += ["--features", "oscore"]
            env = os.environ.copy()
            env["COAPTIC_DEVICE_RUN_ID"] = run_id
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=900)
            case = {"oscore": secured, "run_id": run_id, "command": command, "build_passed": result.returncode == 0,
                    "exit_code": result.returncode, "stdout": result.stdout, "stderr": result.stderr, "runtime_passed": False}
            if result.returncode == 0:
                name = "esp32c3-oscore.elf" if secured else "esp32c3-core.elf"
                firmware = args.output.parent / name
                shutil.copyfile(MANIFEST.parent / "target" / TARGET / "release" / "qualification-esp32", firmware)
                case["firmware"] = str(firmware)
                case["firmware_sha256"] = hashlib.sha256(firmware.read_bytes()).hexdigest()
            report["cases"].append(case)
            print(("PASS build " if case["build_passed"] else "FAIL build ") + ("oscore" if secured else "core"), flush=True)
        report["build_passed"] = all(case["build_passed"] for case in report["cases"])
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return 0 if report["runtime_passed" if args.record else "build_passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
