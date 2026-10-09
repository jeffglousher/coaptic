import argparse
import json
import os
import sqlite3
import statistics
import subprocess
import sys
import time
from pathlib import Path


PHASES = {
    "telemetry_lost_receipt": 68,
    "duplicate": 68,
    "cross_device_denied": 131,
    "operation_content_conflict": 137,
    "desired": 69,
    "applied": 68,
    "restart_receipt": 68,
}
BODIES = {1: b"device-one-telemetry\x00\xff", 2: b"device-two-telemetry\xff\x00"}


def require(value, message):
    if not value:
        raise ValueError(message)


def hex_bytes(value, length=None):
    require(isinstance(value, str) and len(value) % 2 == 0, "invalid hexadecimal field")
    encoded = bytes.fromhex(value)
    require(value == encoded.hex(), "noncanonical hexadecimal field")
    require(length is None or len(encoded) == length, "hexadecimal field length mismatch")
    return encoded


def validate(records, run_id, reverse, expected_before, database):
    require(len(records) == 20, "incomplete or surplus evidence records")
    started = records[0]
    complete = records[-1]
    require(started.get("started") is True and complete.get("complete") is True, "missing start/completion")
    for record in (started, complete):
        require(record["run_id"] == run_id and record["reverse"] is reverse, "run identity/order mismatch")
        require(record["production_providers_qualified"] is False, "unqualified provider claim")
    require(started["hardware_qualified"] is False, "hardware qualification claim")
    require(started["listener_sockets"] == 1 and started["devices"] == 2, "listener/device count mismatch")
    require(started["client_order"] == ([2, 1] if reverse else [1, 2]), "unbalanced schedule")
    require((started["global_datagrams"], started["source_datagrams"], started["global_crypto"], started["source_crypto"]) == (64, 64, 16, 16), "resource/security profile changed")
    require(complete["throughput_comparison"] is False, "unqualified throughput claim")
    require((complete["intentionally_lost_receipts"], complete["old_session_packets_refused"], complete["new_effects"]) == (1, 2, 2), "loss/replay/effect count mismatch")
    require(complete["receipt_count"] == expected_before + 2, "durable effect count mismatch")
    require(0 < complete["wall_ms"] < 45_000 and complete["server_sends"] >= 25, "missing timing/send evidence")
    require(all(complete[field] > 0 for field in ("managed_client_bytes", "managed_server_owner_bytes", "cloud_admission_bytes")), "missing fixed-state memory evidence")
    admissions = {1: [], 2: []}
    phases = {}
    for record in records[1:-1]:
        if record.get("admitted") is True:
            require(record["device"] in admissions, "invalid admitted device")
            hex_bytes(record["principal"], 32)
            hex_bytes(record["resource"], 32)
            hex_bytes(record["local_cid"], 7)
            admissions[record["device"]].append(record)
        else:
            phase = record.get("phase")
            require(phase in PHASES and record["device"] in admissions, "unexpected response")
            key = (phase, record["device"])
            require(key not in phases, "duplicate response evidence")
            phases[key] = record
    require(all(len(entries) == 2 for entries in admissions.values()), "missing initial/restart admission")
    require(len({entry["local_cid"] for entries in admissions.values() for entry in entries}) == 4, "CID reused across retained/live ownership")
    require(len({entries[0]["principal"] for entries in admissions.values()}) == 2, "devices share authenticated principal")
    require(len({entries[0]["resource"] for entries in admissions.values()}) == 2, "devices share resource policy fixture")
    require(len({entry["endpoint"] for entries in admissions.values() for entry in entries}) == 4, "endpoint isolation/restart fixture incomplete")
    for entries in admissions.values():
        require(entries[0]["principal"] == entries[1]["principal"] and entries[0]["resource"] == entries[1]["resource"], "authenticated device policy changed on restart")
    require(len(phases) == 14, "incomplete response phases")
    receipts = {}
    for device in (1, 2):
        operation = device.to_bytes(8, "big") + run_id.to_bytes(8, "big")
        request = operation + BODIES[device]
        tokens = set()
        mids = set()
        for phase, code in PHASES.items():
            record = phases[(phase, device)]
            payload = hex_bytes(record["payload"])
            token = hex_bytes(record["token"], 8)
            local_endpoint = admissions[device][int(phase == "restart_receipt")]["endpoint"]
            require((local_endpoint, token) not in tokens, "request token reused across logical exchanges")
            tokens.add((local_endpoint, token))
            require(isinstance(record["mid"], int) and 0 <= record["mid"] <= 65535, "invalid CoAP MID")
            require(record["mid"] == record["request_mid"], "ACK MID differs from actual transmitted request")
            require((local_endpoint, record["mid"]) not in mids, "MID reused across retained exchanges")
            mids.add((local_endpoint, record["mid"]))
            require(record["code"] == code and record["mtype"] == 2 and record["peer"] == started["server"], "response code/type/endpoint mismatch")
            require(record["source_len"] == len(payload) and record["truncated"] is False, "incomplete payload evidence")
            require(record["content_format"] == (42 if code < 128 else None), "response content format mismatch")
            require(record["target"] == (3 - device if phase == "cross_device_denied" else device), "resource target mismatch")
            if phase in {"telemetry_lost_receipt", "duplicate", "restart_receipt"}:
                require(hex_bytes(record["request_payload"]) == request, "complete telemetry request mismatch")
                require(len(payload) == 56 and payload[:16] == operation and int.from_bytes(payload[48:], "big") > 0, "complete receipt identity/length/sequence mismatch")
                if device in receipts:
                    require(payload == receipts[device], "durable receipt changed across duplicate/restart")
                else:
                    receipts[device] = payload
            elif phase == "operation_content_conflict":
                changed = request[:-1] + bytes([request[-1] ^ 1])
                require(hex_bytes(record["request_payload"]) == changed and not payload, "content-conflict request/response mismatch")
            elif phase == "cross_device_denied":
                require(hex_bytes(record["request_payload"]) == request and not payload, "cross-device refusal changed complete payload")
            elif phase == "desired":
                require(record["request_payload"] == "" and payload == (1).to_bytes(8, "big") + b"device-config-" + bytes([device, 0, 255]), "desired state payload mismatch")
            else:
                require(hex_bytes(record["request_payload"], 8) == (1).to_bytes(8, "big") and payload == (1).to_bytes(8, "big"), "applied state payload mismatch")
    require(receipts[1] != receipts[2], "cross-device receipts conflated")
    with sqlite3.connect(database) as store:
        require(store.execute("SELECT COUNT(*) FROM receipts").fetchone()[0] == expected_before + 2, "database effect count mismatch")
        require(store.execute("SELECT COUNT(*) FROM desired").fetchone()[0] == 2, "database desired capacity mismatch")
        for device in (1, 2):
            principal = hex_bytes(admissions[device][0]["principal"], 32)
            resource = hex_bytes(admissions[device][0]["resource"], 32)
            receipt = receipts[device]
            row = store.execute("SELECT sequence,resource,format,digest,payload FROM receipts WHERE principal=? AND operation=?", (principal, receipt[:16])).fetchone()
            require(row == (int.from_bytes(receipt[48:], "big"), resource, 42, receipt[16:48], BODIES[device]), "database complete payload/security/resource metadata mismatch")
            desired = store.execute("SELECT version,payload,applied FROM desired WHERE principal=? AND resource=?", (principal, resource)).fetchone()
            require(desired == (1, b"device-config-" + bytes([device, 0, 255]), 1), "database desired/applied metadata mismatch")
    return complete


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repetitions", type=int, default=10)
    args = parser.parse_args()
    require(args.repetitions > 0 and args.repetitions % 2 == 0, "use balanced even repetitions")
    root = Path(__file__).resolve().parents[2]
    binary = args.binary.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    database = output / "receipts.sqlite3"
    require(not database.exists(), "choose a fresh evidence directory; prior evidence is preserved")
    environment = os.environ.copy()
    environment["COAPTIC_PYTHON"] = sys.executable
    results = []
    before = 0
    for index in range(args.repetitions):
        run_id = index + 1
        reverse = bool(index % 2)
        command = [str(binary), str(database), str(run_id)] + (["reverse"] if reverse else [])
        wall_started = time.monotonic_ns()
        result = {"run_id": run_id, "reverse": reverse, "command": command}
        stdout, stderr = b"", b""
        try:
            process = subprocess.run(command, cwd=root, env=environment, capture_output=True, timeout=45)
            result["exit_code"] = process.returncode
            stdout, stderr = process.stdout, process.stderr
            require(process.returncode == 0, "fixture process failed")
            records = [json.loads(line) for line in stdout.decode("utf-8").splitlines()]
            result["complete"] = validate(records, run_id, reverse, before, database)
            result["passed"] = True
            before += 2
        except subprocess.TimeoutExpired as error:
            stdout, stderr = error.stdout or b"", error.stderr or b""
            result.update(passed=False, error="fixture process timed out")
        except (OSError, ValueError, KeyError, TypeError, UnicodeError, sqlite3.Error) as error:
            result.update(passed=False, error=str(error))
        result["driver_wall_ms"] = (time.monotonic_ns() - wall_started) / 1_000_000
        (output / f"run-{run_id:02d}.stdout.jsonl").write_bytes(stdout)
        (output / f"run-{run_id:02d}.stderr.log").write_bytes(stderr)
        results.append(result)
        try:
            with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as store:
                before = store.execute("SELECT COUNT(*) FROM receipts").fetchone()[0]
        except sqlite3.Error as error:
            result["database_error"] = str(error)
            result["passed"] = False
        print(json.dumps(result), flush=True)
    walls = [row["complete"]["wall_ms"] for row in results if row.get("passed")]
    report = {
        "repetitions": args.repetitions,
        "passed": sum(bool(row.get("passed")) for row in results),
        "failed": sum(not row.get("passed") for row in results),
        "schedule_counts": {"first_device_first": args.repetitions // 2, "second_device_first": args.repetitions // 2},
        "raw_wall_ms": walls,
        "raw_driver_wall_ms_including_failures": [row["driver_wall_ms"] for row in results],
        "median_wall_ms": statistics.median(walls) if walls else None,
        "minimum_wall_ms": min(walls) if walls else None,
        "maximum_wall_ms": max(walls) if walls else None,
        "results": results,
        "database": str(database),
        "durable_receipts": before,
        "configured_receipt_losses_per_fixture": 1,
        "confirmed_intentionally_lost_receipts": sum(row["complete"]["intentionally_lost_receipts"] for row in results if row.get("passed")),
        "production_providers_qualified": False,
        "hardware_qualified": False,
        "throughput_comparison": False,
        "cpu_utilization_measured": False,
        "outliers_removed": False,
    }
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: value for key, value in report.items() if key != "results"}), flush=True)
    return int(report["failed"] != 0)


if __name__ == "__main__":
    raise SystemExit(main())
