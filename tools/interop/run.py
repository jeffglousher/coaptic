#!/usr/bin/env python3
"""Process-isolated interop, request timing and deterministic UDP fault checks.

No third-party Python dependencies. This is a smoke/measurement harness, not a
claim of complete ETSI coverage or a statistically controlled performance SLA.
"""
import argparse
import hashlib
import json
import math
import platform
import queue
import select
import socket
import statistics
import subprocess
import tempfile
import threading
import time
from pathlib import Path

SCHEMA = "coaptic-peer/2"
BODY = b"core-test-payload"
LARGE = bytes(i % 251 for i in range(2000))


def port():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def command(exe, role, transport, number, key="sesame", path="test", method="GET", timeout=6000):
    return [str(exe), role, transport, str(number), key, path, method, str(timeout)]


def decode(line):
    if len(line) > 65536:
        raise RuntimeError("oversize peer event")
    value = json.loads(line)
    if value.get("schema") != SCHEMA:
        raise RuntimeError("peer schema mismatch")
    return value


class Server:
    def __init__(self, exe, transport, number=None):
        self.number = number or port()
        self.stderr = tempfile.TemporaryFile()
        start = time.perf_counter_ns()
        self.proc = subprocess.Popen(command(exe, "server", transport, self.number),
                                     stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                     stderr=self.stderr)
        events = queue.Queue(maxsize=1)
        def read_ready():
            try:
                events.put(self.proc.stdout.readline(65537))
            except Exception as error:
                events.put(error)
        self.reader = threading.Thread(target=read_ready, daemon=True)
        self.reader.start()
        try:
            line = events.get(timeout=8)
            if isinstance(line, Exception):
                raise line
            self.ready = decode(line)
            if self.ready.get("event") != "ready" or self.ready.get("port") != self.number or self.ready.get("transport") != transport:
                raise RuntimeError(f"server did not become ready: {self.ready}")
            if self.proc.poll() is not None:
                raise RuntimeError("server exited during readiness")
            self.startup_ms = (time.perf_counter_ns() - start) / 1_000_000
        except Exception:
            self.close()
            raise

    def close(self):
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait(timeout=2)
        self.reader.join(timeout=1)
        self.proc.stdout.close()
        self.stderr.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


def request(exe, transport, number, **kwargs):
    start = time.perf_counter_ns()
    timeout = kwargs.get("timeout", 6000)
    try:
        result = subprocess.run(command(exe, "client", transport, number, **kwargs),
                                capture_output=True, timeout=timeout / 1000 + 4)
    except subprocess.TimeoutExpired as error:
        raise RuntimeError("peer exceeded process deadline (not a valid protocol timeout)") from error
    host_ns = time.perf_counter_ns() - start
    if len(result.stdout) > 65536:
        raise RuntimeError("oversize peer output")
    lines = result.stdout.splitlines()
    if len(lines) != 1:
        raise RuntimeError(f"expected one peer event, exit={result.returncode}, stderr={result.stderr[-2000:]!r}, stdout={result.stdout[:1000]!r}")
    event = decode(lines[0])
    event["host_total_ns"] = host_ns
    event["host_total_us"] = host_ns / 1000
    if result.returncode == 0:
        if event.get("event") != "response":
            raise RuntimeError("successful process without response")
        validate_timing(event)
    elif event.get("event") != "error":
        raise RuntimeError(f"peer crashed or contradicted response: {event}")
    event["exit_code"] = result.returncode
    return event


def validate_timing(event):
    # Reject bools, NaN, fractions, missing/old timing and malformed clock data.
    elapsed = event.get("elapsed_ns")
    clock = event.get("clock")
    if type(elapsed) is not int or not 0 <= elapsed <= 60_000_000_000:
        raise RuntimeError("invalid request nanosecond timing")
    if not isinstance(clock, dict) or not isinstance(clock.get("name"), str) or not clock["name"]:
        raise RuntimeError("missing request clock metadata")
    if "resolution_ns" not in clock:
        raise RuntimeError("missing clock resolution (null means unknown)")
    resolution = clock["resolution_ns"]
    if resolution is not None and (type(resolution) is not int or resolution <= 0):
        raise RuntimeError("invalid clock resolution")
    return elapsed


def expect(event, code=69, payload=BODY):
    if event["exit_code"] != 0 or event.get("code") != code:
        raise AssertionError(f"expected code {code}: {event}")
    if payload is not None and bytes.fromhex(event.get("payload_hex", "")) != payload:
        raise AssertionError(f"incorrect response body: {event}")


class Proxy:
    """One-client UDP relay, deterministic first-packet faults, bounded trace."""
    def __init__(self, dest, mode):
        self.front = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.back = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.front.bind(("127.0.0.1", 0))
        self.back.bind(("127.0.0.1", 0))
        self.number = self.front.getsockname()[1]
        self.dest = ("127.0.0.1", dest)
        self.mode, self.trace, self.error = mode, [], None
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        client = None
        dropped = duplicated = False
        try:
            while not self.stop.is_set():
                ready, _, _ = select.select([self.front, self.back], [], [], .02)
                for sock in ready:
                    try:
                        data, source = sock.recvfrom(65536)
                    except ConnectionResetError:
                        # Windows reports late ICMP port-unreachable when a
                        # one-shot client exits after the first duplicate reply.
                        if self.mode == "duplicate-request" and sock is self.front:
                            continue
                        raise
                    incoming = sock is self.front
                    if not incoming and source != self.dest:
                        raise RuntimeError("unexpected proxy upstream")
                    action = "forward"
                    if incoming:
                        if client is not None and source != client:
                            raise RuntimeError("multiple clients in single-request fault proxy")
                        client = source
                    if self.mode == "blackhole":
                        action = "drop"
                    elif self.mode == "drop-reply" and not incoming and not dropped:
                        action, dropped = "drop", True
                    elif self.mode == "duplicate-request" and incoming and not duplicated:
                        action, duplicated = "duplicate", True
                    if len(self.trace) >= 256:
                        raise RuntimeError("proxy trace limit exceeded")
                    self.trace.append({"direction": "request" if incoming else "response",
                                       "action": action, "hex": data.hex()})
                    if action != "drop":
                        target = self.back if incoming else self.front
                        address = self.dest if incoming else client
                        target.sendto(data, address)
                        if action == "duplicate":
                            target.sendto(data, address)
        except Exception as error:
            self.error = str(error)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.stop.set()
        self.thread.join(timeout=1)
        self.front.close()
        self.back.close()
        if self.thread.is_alive() or self.error:
            raise RuntimeError(f"proxy failed: {self.error}")


def summary(samples):
    ordered = sorted(samples)
    return {"n": len(samples), "min": min(samples), "mean": statistics.mean(samples),
            "p50": ordered[math.ceil(.5 * len(samples)) - 1],
            "p95": ordered[math.ceil(.95 * len(samples)) - 1],
            "p99": ordered[math.ceil(.99 * len(samples)) - 1], "max": max(samples)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--coaptic", required=True, type=Path)
    parser.add_argument("--coap-rs", required=True, type=Path)
    parser.add_argument("--libcoap", required=True, type=Path)
    parser.add_argument("--libcoap-udp-only", action="store_true", help="Explicit local build limitation, recorded in results; CI requires DTLS")
    parser.add_argument("--iterations", type=int, default=100)
    parser.add_argument("--build-note", default="Unspecified build profiles; do not compare timings across peers")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if not 1 <= args.iterations <= 10000:
        parser.error("iterations must be 1..10000")
    peers = {"coaptic": args.coaptic.resolve(), "coap-rs": args.coap_rs.resolve(), "libcoap": args.libcoap.resolve()}
    report = {"schema": "coaptic-process-interop/2", "platform": platform.platform(),
              "source": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
              "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], text=True).strip()),
              "iterations": args.iterations, "build_note": args.build_note, "timing_scope": "child request includes socket/session/DTLS handshake and response assembly; excludes process startup. host_total includes spawn and exit. Serial, fresh client per request; no warm-session throughput claim.",
              "host_clock": {"name": time.get_clock_info("perf_counter").implementation, "resolution_ns": math.ceil(time.get_clock_info("perf_counter").resolution * 1e9)},
              "libcoap_source": "7cf7465b784baded4de183290c547d582becfd28",
              "limitations": ["PSK DTLS only; OSCORE/Observe/certificate scenarios remain in the existing harness; no claim of full ETSI coverage"],
              "executables": {n: {"path": str(p), "sha256": hashlib.sha256(p.read_bytes()).hexdigest()} for n,p in peers.items()},
              "cases": [], "benchmarks": []}
    if args.libcoap_udp_only:
        report["limitations"].append("libcoap DTLS explicitly excluded for this local build")

    def case(name, function):
        try:
            detail = function()
            report["cases"].append({"name": name, "passed": True, "evidence": detail})
            print(f"PASS {name}", flush=True)
        except Exception as error:
            report["cases"].append({"name": name, "passed": False, "error": str(error)})
            print(f"FAIL {name}: {error}", flush=True)

    for transport in ("udp", "dtls"):
        pairs = [("coaptic", "coaptic"), ("coaptic", "coap-rs"), ("coap-rs", "coaptic"), ("coaptic", "libcoap"), ("libcoap", "coaptic")]
        for client, server in pairs:
            if transport == "dtls" and args.libcoap_udp_only and "libcoap" in (client, server):
                continue
            label = f"{transport}:{client}->{server}"
            def matrix(client=client, server=server, transport=transport, label=label):
                with Server(peers[server], transport) as service:
                    # Warm-up is an actual request; JSON readiness alone does not prove service health.
                    expect(request(peers[client], transport, service.number))
                    expect(request(peers[client], transport, service.number, path="missing"), 132, None)
                    expect(request(peers[client], transport, service.number, path="large"), 69, LARGE)
                    samples, host = [], []
                    clock = None
                    wall = time.perf_counter()
                    for _ in range(args.iterations):
                        event = request(peers[client], transport, service.number)
                        expect(event)
                        if clock is not None and event["clock"] != clock:
                            raise RuntimeError("request clock changed within benchmark")
                        clock = event["clock"]
                        samples.append(event["elapsed_ns"])
                        host.append(event["host_total_ns"])
                    elapsed = time.perf_counter() - wall
                    report["benchmarks"].append({"pair": label, "server": service.ready,
                        "startup_ms": service.startup_ms, "clock": clock,
                        "samples_ns": {"request": samples, "host_total": host},
                        "request_ns": summary(samples), "host_total_ns": summary(host),
                        "request_us": summary([n / 1000 for n in samples]),
                        "host_total_us": summary([n / 1000 for n in host]), "serial_host_requests_per_second": args.iterations / elapsed,
                        "failures": 0})
                    if transport == "dtls":
                        refused = request(peers[client], transport, service.number, key="incorrect", timeout=1500)
                        if refused["exit_code"] == 0 or not any(s in refused.get("message", "").lower() for s in ("timeout", "timed out", "deadline", "handshake", "decrypt", "alert", "elapsed")):
                            raise AssertionError(f"wrong key did not produce authentication failure/timeout: {refused}")
                        # A failed handshake must not destroy availability.
                        expect(request(peers[client], transport, service.number))
                    return {"server": service.ready, "verified": ["GET bytes", "4.04", "2000-byte Block2", "repeat requests"]}
            case(label, matrix)

    # Fault tests target Coaptic's guarantees; alternative clients remain independent.
    for client in peers:
        for mode in ("drop-reply", "duplicate-request", "blackhole"):
            def fault(client=client, mode=mode):
                with Server(peers["coaptic"], "udp") as service:
                    with Proxy(service.number, mode) as relay:
                        event = request(peers[client], "udp", relay.number,
                                        path="counter" if mode != "blackhole" else "test",
                                        method="POST" if mode != "blackhole" else "GET",
                                        timeout=6500 if mode != "blackhole" else 500)
                    if mode == "blackhole":
                        if event["exit_code"] == 0 or not any(s in event.get("message", "").lower() for s in ("timed out", "timeout", "elapsed")):
                            raise AssertionError(f"blackhole did not return bounded timeout: {event}")
                    else:
                        expect(event, 68, b"")
                        expect(request(peers[client], "udp", service.number, path="counter"), 69, b"1")
                        if mode == "drop-reply" and sum(x["direction"] == "request" for x in relay.trace) < 2:
                            raise AssertionError("no observed retransmission")
                    if not relay.trace or not any(x["action"] != "forward" for x in relay.trace):
                        raise AssertionError("fault was not exercised")
                    return {"trace": relay.trace, "result": event}
            case(f"reliability:{client}->{mode}", fault)
        def restart(client=client):
            number = port()
            with Server(peers["coaptic"], "udp", number) as service:
                expect(request(peers[client], "udp", service.number))
                expect(request(peers[client], "udp", service.number, path="counter", method="POST"), 68, b"")
            with Server(peers["coaptic"], "udp", number) as service:
                expect(request(peers[client], "udp", service.number))
                expect(request(peers[client], "udp", service.number, path="counter"), 69, b"0")
            return {"port": number, "note": "fresh in-memory fixture after process restart; no durability claim"}
        case(f"reliability:{client}->restart", restart)
    report["passed"] = all(c["passed"] for c in report["cases"])
    report["failures"] = sum(not c["passed"] for c in report["cases"])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
