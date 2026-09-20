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
from capabilities import load_manifest, evaluate

SCHEMA = "coaptic-peer/2"
BODY = b"core-test-payload"
LARGE = bytes(i % 251 for i in range(2000))


def port(family="ipv4"):
    with socket.socket(socket.AF_INET6 if family == "ipv6" else socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.bind(("::1" if family == "ipv6" else "127.0.0.1", 0))
        return sock.getsockname()[1]


def ipv6_probe(number):
    # Independent, hand-written CON GET /test proves the server is reachable
    # at ::1. A peer silently falling back to IPv4 cannot satisfy this probe.
    wire = b"\x41\x01\x76\x60\x8d\xb4test"
    with socket.socket(socket.AF_INET6, socket.SOCK_DGRAM) as sock:
        sock.bind(("::1", 0))
        sock.settimeout(2)
        sock.sendto(wire, ("::1", number))
        reply, sender = sock.recvfrom(4096)
    if sender[0] != "::1" or sender[1] != number or reply[:5] != b"\x61\x45\x76\x60\x8d" or not reply.endswith(b"\xff" + BODY):
        raise AssertionError("IPv6 server probe response mismatch")
    return {"sender": sender, "request_hex": wire.hex(), "response_hex": reply.hex()}


def command(exe, role, transport, number, key="sesame", path="test", method="GET", timeout=6000, family="ipv4", payload=b""):
    return [str(exe), role, transport, str(number), key, path, method, str(timeout), family, payload.hex()]


def decode(line):
    if len(line) > 65536:
        raise RuntimeError("oversize peer event")
    value = json.loads(line)
    if value.get("schema") != SCHEMA:
        raise RuntimeError("peer schema mismatch")
    return value


class Server:
    def __init__(self, exe, transport, number=None, family="ipv4"):
        self.number = number or port(family)
        self.stderr = tempfile.TemporaryFile()
        start = time.perf_counter_ns()
        self.proc = subprocess.Popen(command(exe, "server", transport, self.number, family=family),
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


def expect_refusal(event, *, handshake=False):
    words = ("timeout", "timed out", "deadline", "elapsed")
    if handshake:
        words += ("handshake", "decrypt", "alert")
    if type(event.get("exit_code")) is not int or event["exit_code"] != 1 or event.get("event") != "error" or not any(
            word in event.get("message", "").lower() for word in words):
        raise AssertionError(f"not a bounded protocol refusal/timeout: {event}")


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
    def __init__(self, dest, mode, family="ipv4"):
        if family not in ("ipv4", "ipv6"):
            raise ValueError("unsupported proxy address family")
        address_family = socket.AF_INET6 if family == "ipv6" else socket.AF_INET
        address = "::1" if family == "ipv6" else "127.0.0.1"
        self.front = socket.socket(address_family, socket.SOCK_DGRAM)
        self.back = socket.socket(address_family, socket.SOCK_DGRAM)
        for endpoint in (self.front, self.back):
            if family == "ipv6":
                endpoint.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
            endpoint.bind((address, 0))
        self.number = self.front.getsockname()[1]
        self.dest = (address, dest, 0, 0) if family == "ipv6" else (address, dest)
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
                        if self.mode in ("duplicate-request", "dtls-reconnect") and sock is self.front:
                            continue
                        raise
                    incoming = sock is self.front
                    if not incoming and source != self.dest:
                        raise RuntimeError("unexpected proxy upstream")
                    action = "forward"
                    if incoming:
                        if client is not None and source != client and self.mode != "dtls-reconnect":
                            raise RuntimeError("multiple clients in single-request fault proxy")
                        client = source
                    if self.mode == "blackhole":
                        action = "drop"
                    elif self.mode == "drop-reply" and not incoming and not dropped:
                        action, dropped = "drop", True
                    elif self.mode == "duplicate-request" and incoming and not duplicated:
                        action, duplicated = "duplicate", True
                    if len(self.trace) >= (8192 if self.mode == "dtls-reconnect" else 256):
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


def expect_identical_requests(trace, require_repeat=False):
    requests = [row["hex"] for row in trace if row["direction"] == "request"]
    if len(requests) < (2 if require_repeat else 1) or len(set(requests)) != 1:
        raise AssertionError("missing or changed retransmitted request bytes")


def summary(samples):
    ordered = sorted(samples)
    return {"n": len(samples), "min": min(samples), "mean": statistics.mean(samples),
            "p50": ordered[math.ceil(.5 * len(samples)) - 1],
            "p95": ordered[math.ceil(.95 * len(samples)) - 1],
            "p99": ordered[math.ceil(.99 * len(samples)) - 1], "max": max(samples)}


def measure_requests(iterations, request_fn):
    samples, host, failures = [], [], []
    clock = None
    started = time.perf_counter_ns()
    for index in range(iterations):
        try:
            event = request_fn()
            expect(event)
            validate_timing(event)
            if type(event.get("host_total_ns")) is not int or event["host_total_ns"] < 0:
                raise RuntimeError("invalid host timing")
            if clock is not None and event["clock"] != clock:
                raise RuntimeError("request clock changed within benchmark")
            clock = event["clock"]
            samples.append(event["elapsed_ns"])
            host.append(event["host_total_ns"])
        except Exception as error:
            failures.append({"sample_index": index, "error": str(error)})
            break  # No retries or successful-sample substitution.
    wall_ns = time.perf_counter_ns() - started
    result = {"clock": clock, "requested_samples": iterations,
              "samples_ns": {"request": samples, "host_total": host},
              "failures": len(failures), "sample_failures": failures}
    if samples:
        result.update({"request_ns": summary(samples), "host_total_ns": summary(host),
                       "request_us": summary([n / 1000 for n in samples]),
                       "host_total_us": summary([n / 1000 for n in host])})
    if not failures:
        result["serial_host_requests_per_second"] = iterations * 1e9 / wall_ns
    return result


def method_workflow(client, server, transport="udp", family="ipv4"):
    """Literal byte/state oracles; private binary patch syntax, no JSON Patch claim."""
    steps = [
        ("GET", b"", 132, b""),
        ("PUT", b"alpha", 65, b""), ("GET", b"", 69, b"alpha"),
        ("PUT", b"a", 68, b""), ("GET", b"", 69, b"a"),
        ("POST", b"b", 68, b""), ("GET", b"", 69, b"ab"),
        ("PATCH", b"+c", 68, b""), ("GET", b"", 69, b"abc"),
        ("PATCH", b"+c", 68, b""), ("GET", b"", 69, b"abcc"),
        ("IPATCH", b"=final", 68, b""), ("GET", b"", 69, b"final"),
        ("IPATCH", b"=final", 68, b""), ("GET", b"", 69, b"final"),
        ("FETCH", b"value", 69, b"final"),
        ("FETCH", b"wrong", 128, None), ("GET", b"", 69, b"final"),
        ("PATCH", b"wrong", 128, None), ("GET", b"", 69, b"final"),
        ("IPATCH", b"wrong", 128, None), ("GET", b"", 69, b"final"),
        ("PUT", b"x" * 65, 141, None), ("GET", b"", 69, b"final"),
        ("POST", b"x" * 60, 141, None), ("GET", b"", 69, b"final"),
        ("DELETE", b"", 66, b""), ("GET", b"", 132, None),
        ("DELETE", b"", 132, None),
    ]
    evidence = []
    with Server(server, transport, family=family) as service:
        probe = ipv6_probe(service.number) if family == "ipv6" else None
        refusal = None
        for index, (method, payload, code, body) in enumerate(steps):
            if transport == "dtls" and index == 2:
                # A refused replacement must leave the accepted PUT intact.
                refusal = request(client, transport, service.number, family=family, path="methods",
                                  method="PUT", payload=b"poison", key="incorrect", timeout=1500)
                expect_refusal(refusal, handshake=True)
            result = request(client, transport, service.number, family=family, path="methods", method=method, payload=payload)
            expect(result, code, body)
            evidence.append({"method": method, "request_hex": payload.hex(), "expected_code": code,
                             "expected_payload_hex": None if body is None else body.hex(), "response": result})
    return {"steps": evidence, "transport": transport, "address_family": family,
            "ipv6_socket_probe": probe, "wrong_key_result": refusal, "state_limit_bytes": 64, "patch_format": "fixture octet-stream: PATCH +suffix; IPATCH =replacement"}


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
    manifest, manifest_hash = load_manifest()
    report = {"schema": "coaptic-process-interop/2", "platform": platform.platform(),
              "source": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
              "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], text=True).strip()),
              "iterations": args.iterations, "build_note": args.build_note, "timing_scope": "child request includes socket/session/DTLS handshake and response assembly; excludes process startup. host_total includes spawn and exit. Serial, fresh client per request; no warm-session throughput claim.",
              "host_clock": {"name": time.get_clock_info("perf_counter").implementation, "resolution_ns": math.ceil(time.get_clock_info("perf_counter").resolution * 1e9)},
              "libcoap_source": "7cf7465b784baded4de183290c547d582becfd28",
              "limitations": ["PSK DTLS only; this suite does not qualify OSCORE, Observe or certificates; no claim of full ETSI coverage"],
              "executables": {n: {"path": str(p), "sha256": hashlib.sha256(p.read_bytes()).hexdigest()} for n,p in peers.items()},
              "capability_manifest": {"path": "tools/interop/capabilities.json", "sha256": manifest_hash},
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
                    measured = measure_requests(args.iterations,
                        lambda: request(peers[client], transport, service.number))
                    report["benchmarks"].append({"pair": label, "server": service.ready,
                        "startup_ms": service.startup_ms, **measured})
                    if measured["failures"]:
                        raise AssertionError(f"request measurement failed: {measured['sample_failures']}")
                    if transport == "dtls":
                        refused = request(peers[client], transport, service.number, key="incorrect", timeout=1500)
                        expect_refusal(refused, handshake=True)
                        # A failed handshake must not destroy availability.
                        expect(request(peers[client], transport, service.number))
                    return {"server": service.ready, "verified": ["GET bytes", "4.04", "2000-byte Block2", "repeat requests"]}
            case(label, matrix)

    for transport, family, label in (("udp", "ipv4", "methods-udp"),
                                     ("dtls", "ipv4", "methods-dtls"),
                                     ("udp", "ipv6", "methods-ipv6-udp")):
        for client, server in [("coaptic", "coaptic"), ("coaptic", "coap-rs"), ("coap-rs", "coaptic"), ("coaptic", "libcoap"), ("libcoap", "coaptic")]:
            if transport == "dtls" and args.libcoap_udp_only and "libcoap" in (client, server):
                continue
            case(f"{label}:{client}->{server}",
                 lambda client=client, server=server, transport=transport, family=family:
                 method_workflow(peers[client], peers[server], transport, family))

    for client, server in [("coaptic", "coaptic"), ("coaptic", "coap-rs"), ("coap-rs", "coaptic"), ("coaptic", "libcoap"), ("libcoap", "coaptic")]:
        def ipv6_matrix(client=client, server=server):
            with Server(peers[server], "udp", family="ipv6") as service:
                probe = ipv6_probe(service.number)
                expect(request(peers[client], "udp", service.number, family="ipv6"))
                expect(request(peers[client], "udp", service.number, family="ipv6", path="missing"), 132, None)
                expect(request(peers[client], "udp", service.number, family="ipv6", path="large"), 69, LARGE)
                expect(request(peers[client], "udp", service.number, family="ipv6"))
                return {"server": service.ready, "address": "::1", "ipv6_socket_probe": probe, "verified": ["IPv6 UDP GET bytes", "4.04", "2000-byte Block2", "service after refusal"]}
        case(f"ipv6-udp:{client}->{server}", ipv6_matrix)

    for client, server in [("coaptic", "coaptic"), ("coaptic", "coap-rs"), ("coap-rs", "coaptic"), ("coaptic", "libcoap"), ("libcoap", "coaptic")]:
        if args.libcoap_udp_only and "libcoap" in (client, server):
            continue
        def ipv6_dtls(client=client, server=server):
            with Server(peers[server], "dtls", family="ipv6") as service:
                # New relay per session: qualify IPv6 transport without assuming
                # a peer's same-endpoint replacement-handshake policy.
                traces = []
                def exchange(**kwargs):
                    with Proxy(service.number, "dtls-reconnect", family="ipv6") as relay:
                        result = request(peers[client], "dtls", relay.number, family="ipv6", **kwargs)
                    if not any(row["direction"] == "request" for row in relay.trace):
                        raise AssertionError("no IPv6 request datagrams")
                    traces.append(relay.trace)
                    return result
                expect(exchange())
                expect(exchange(path="missing"), 132, None)
                expect(exchange(path="large"), 69, LARGE)
                refused = exchange(key="incorrect", timeout=1500)
                expect_refusal(refused, handshake=True)
                expect(exchange())
                return {"server": service.ready, "address": "::1", "relay_family": "AF_INET6",
                        "ipv6_only": True, "traces": traces, "wrong_key_result": refused,
                        "verified": ["IPv6 PSK DTLS GET bytes", "4.04", "2000-byte Block2", "wrong-key refusal", "service after refusal"]}
        case(f"ipv6-dtls:{client}->{server}", ipv6_dtls)

    # The relay preserves one server-visible UDP endpoint across fresh clients.
    for server in ("coaptic", "coap-rs"):
        for client in peers:
            if client == "libcoap" and args.libcoap_udp_only:
                continue
            def reconnect(server=server, client=client):
                with Server(peers[server], "dtls") as service:
                    with Proxy(service.number, "dtls-reconnect") as relay:
                        for _ in range(3):
                            expect(request(peers[client], "dtls", relay.number))
                        refused = request(peers[client], "dtls", relay.number, key="incorrect", timeout=1500)
                        expect_refusal(refused, handshake=True)
                        expect(request(peers[client], "dtls", relay.number))
                        endpoint = relay.back.getsockname()
                return {"server_endpoint": endpoint, "successful_connections": 4,
                        "wrong_key_result": refused, "trace": relay.trace}
            case(f"dtls-reconnect:{client}->{server}", reconnect)
        def churn(server=server):
            with Server(peers[server], "dtls") as service:
                with Proxy(service.number, "dtls-reconnect") as relay:
                    for _ in range(140):
                        expect(request(peers["coaptic"], "dtls", relay.number, path="counter", method="POST"), 68, b"")
                    expect(request(peers["coaptic"], "dtls", relay.number, path="counter"), 69, b"140")
                    endpoint = relay.back.getsockname()
            return {"server_endpoint": endpoint, "connections": 141,
                    "verified": "140 distinct authenticated POST effects across reused endpoint", "trace": relay.trace}
        case(f"dtls-churn:coaptic->{server}", churn)

    if not args.libcoap_udp_only:
        def c_server_reconnect():
            with Server(peers["libcoap"], "dtls") as service:
                with Proxy(service.number, "dtls-reconnect") as relay:
                    for _ in range(140):
                        expect(request(peers["coaptic"], "dtls", relay.number, path="counter", method="POST"), 68, b"")
                    expect(request(peers["coaptic"], "dtls", relay.number, path="counter"), 69, b"140")
                    endpoint = relay.back.getsockname()
                    alerts = sum(row["direction"] == "request" and row["hex"].startswith("15") for row in relay.trace)
                    if alerts < 141:
                        raise AssertionError(f"missing terminal client records: {alerts}")
            return {"server_endpoint": endpoint, "connections": 141, "client_alert_records": alerts,
                    "verified": "Coaptic explicit shutdown and 140 independent POST effects", "trace": relay.trace}
        case("dtls-clean-reconnect:coaptic->libcoap", c_server_reconnect)

    # Exercise both Coaptic roles against independent peer implementations.
    for client, server in [(name, "coaptic") for name in peers] + [("coaptic", "coap-rs"), ("coaptic", "libcoap")]:
        label = client if server == "coaptic" else f"{client}->{server}"
        for mode in ("drop-reply", "duplicate-request", "blackhole"):
            def fault(client=client, server=server, mode=mode):
                with Server(peers[server], "udp") as service:
                    with Proxy(service.number, mode) as relay:
                        idempotent = server != "coaptic"
                        event = request(peers[client], "udp", relay.number,
                                        path=("methods" if idempotent else "counter") if mode != "blackhole" else "test",
                                        method=("PUT" if idempotent else "POST") if mode != "blackhole" else "GET",
                                        payload=b"once" if idempotent and mode != "blackhole" else b"",
                                        timeout=6500 if mode != "blackhole" else 500)
                    if mode == "blackhole":
                        expect_refusal(event)
                    else:
                        if idempotent:
                            # A cached Created or a reprocessed Changed are valid
                            # PUT outcomes. Neither proves server deduplication.
                            if event.get("code") not in (65, 68):
                                raise AssertionError("PUT did not create or replace the resource")
                            expect(event, event["code"], b"")
                            readback = request(peers[client], "udp", service.number, path="methods")
                            expect(readback, 69, b"once")
                        else:
                            expect(event, 68, b"")
                            readback = request(peers[client], "udp", service.number, path="counter")
                            expect(readback, 69, b"1")
                        expect_identical_requests(relay.trace, require_repeat=mode == "drop-reply")
                    if not relay.trace or not any(x["action"] != "forward" for x in relay.trace):
                        raise AssertionError("fault was not exercised")
                    return {"trace": relay.trace, "result": event,
                            "readback": None if mode == "blackhole" else readback,
                            "effect_oracle": "idempotent PUT state" if idempotent else "single POST effect"}
            case(f"reliability:{label}->{mode}", fault)
        def restart(client=client, server=server):
            number = port()
            with Server(peers[server], "udp", number) as service:
                expect(request(peers[client], "udp", service.number))
                expect(request(peers[client], "udp", service.number, path="counter", method="POST"), 68, b"")
            with Server(peers[server], "udp", number) as service:
                expect(request(peers[client], "udp", service.number))
                expect(request(peers[client], "udp", service.number, path="counter"), 69, b"0")
            return {"port": number, "note": "fresh in-memory fixture after process restart; no durability claim"}
        case(f"reliability:{label}->restart", restart)
    report["coverage"] = evaluate(manifest, report["cases"],
        libcoap_dtls=not args.libcoap_udp_only, system=platform.system().lower())
    report["passed"] = report["coverage"]["complete"]
    report["failures"] = len(report["coverage"]["problems"])
    for problem in report["coverage"]["problems"]:
        print(f"FAIL coverage: {problem}", flush=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
