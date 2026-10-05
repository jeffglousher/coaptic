import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
TOOLCHAIN = "nightly-2026-10-01"
TARGETS = ("datagram", "cbor", "oscore")
SEEDS = {"datagram": bytes.fromhex("41011234abb178"),
         "cbor": bytes.fromhex("a1206161"), "oscore": bytes.fromhex("000000000161")}


def evidence(stderr):
    counters = re.findall(r"cov: (\d+) ft: (\d+)", stderr)
    executions = re.findall(r"stat::number_of_executed_units:\s*(\d+)", stderr)
    if not counters or not executions or int(executions[-1]) < 100:
        raise ValueError("missing coverage feedback or executed-input evidence")
    coverage, features = map(int, counters[-1])
    if coverage == 0 or features == 0:
        raise ValueError("empty coverage feedback")
    return {"coverage_points": coverage, "features": features, "executed_inputs": int(executions[-1])}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=int, default=60)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.seconds < 1:
        parser.error("--seconds must be positive")
    os.chdir(ROOT)
    report = {"schema": "coaptic-fuzz/1", "passed": False,
              "source": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
              "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], text=True).strip()),
              "compiler": subprocess.check_output(["rustc", "+" + TOOLCHAIN, "--version", "--verbose"], text=True),
              "cargo_fuzz": subprocess.check_output(["cargo", "fuzz", "--version"], text=True).strip(),
              "scope": "Coverage-guided libFuzzer campaigns with address sanitizer and semantic oracles",
              "unqualified": ["exhaustive inputs", "RFC requirement completeness", "state-machine lifecycle fuzzing", "device execution"],
              "campaigns": []}
    for target in TARGETS:
        corpus = ROOT / "fuzz" / "corpus" / target
        corpus.mkdir(parents=True, exist_ok=True)
        data = SEEDS[target]
        (corpus / hashlib.sha256(data).hexdigest()).write_bytes(data)
        command = ["cargo", "+" + TOOLCHAIN, "fuzz", "run", target, str(corpus), "--",
                   f"-max_total_time={args.seconds}", "-max_len=4096", "-timeout=10", "-seed=9177", "-print_final_stats=1"]
        row = {"target": target, "command": command, "passed": False}
        try:
            result = subprocess.run(command, capture_output=True, text=True, timeout=args.seconds + 900)
            row.update(exit_code=result.returncode, stdout=result.stdout, stderr=result.stderr)
            if result.returncode == 0:
                row["evidence"] = evidence(result.stderr)
                row["passed"] = True
        except (OSError, ValueError, subprocess.TimeoutExpired) as error:
            row["error"] = str(error)
        report["campaigns"].append(row)
        print(("PASS " if row["passed"] else "FAIL ") + target, flush=True)
    report["passed"] = len(report["campaigns"]) == len(TARGETS) and all(row["passed"] for row in report["campaigns"])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
