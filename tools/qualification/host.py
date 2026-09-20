"""Pinned host library execution evidence; no transport or device qualification."""
import argparse
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
TOOLCHAIN = "1.97.1"
CASES = (
    ("core", ["--no-default-features"]),
    ("all", ["--all-features"]),
    ("alloc", ["--no-default-features", "--features", "alloc"]),
    ("oscore", ["--no-default-features", "--features", "oscore"]),
    ("alloc,oscore", ["--no-default-features", "--features", "alloc,oscore"]),
    ("std", ["--no-default-features", "--features", "std"]),
)
SUMMARY = re.compile(r"test result: ok\. (\d+) passed; 0 failed; (\d+) ignored;")


def text(value):
    return value.decode("utf-8", errors="replace") if isinstance(value, bytes) else (value or "")


def run_case(name, flags, execute=subprocess.run):
    command = ["cargo", "+" + TOOLCHAIN, "test", "--locked", "-p", "coaptic", *flags]
    timed_out = False
    try:
        result = execute(command, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=900)
        code, stdout, stderr = result.returncode, result.stdout, result.stderr
    except subprocess.TimeoutExpired as error:
        code, stdout, stderr, timed_out = None, text(error.stdout), text(error.stderr), True
    except OSError as error:
        code, stdout, stderr = None, "", str(error)
    summaries = [(int(passed), int(ignored)) for passed, ignored in SUMMARY.findall(stdout)]
    executed = sum(passed for passed, _ in summaries)
    passed = code == 0 and executed > 0 and "test result: FAILED" not in stdout
    return {"features": name, "command": command, "passed": passed, "exit_code": code,
            "timed_out": timed_out, "executed": executed, "ignored": sum(ignored for _, ignored in summaries),
            "stdout": stdout, "stderr": stderr}


def run_matrix(execute=subprocess.run):
    cases = []
    for name, flags in CASES:
        result = run_case(name, flags, execute)
        cases.append(result)
        print(("PASS" if result["passed"] else "FAIL") + " " + name, flush=True)
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    os.chdir(ROOT)
    report = {"schema": "coaptic-host-qualification/1",
              "source": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
              "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], text=True).strip()),
              "compiler": subprocess.check_output(["rustc", "+" + TOOLCHAIN, "--version", "--verbose"], text=True),
              "platform": platform.platform(), "machine": platform.machine(), "python": sys.version,
              "scope": "Library unit, integration and rustdoc execution in six feature configurations on this host",
              "unqualified": ["independent process/DTLS adapters on this host", "MSRV on this host", "device execution", "target stack high-water", "full RFC or branch coverage"],
              "cases": run_matrix()}
    report["passed"] = len(report["cases"]) == len(CASES) and all(case["passed"] for case in report["cases"])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
