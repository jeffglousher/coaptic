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
UPLOAD_4K = bytes(i % 251 for i in range(4096))


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


def command(exe, role, transport, number, key="sesame", path="test", method="GET", timeout=6000, family="ipv4", payload=b"", sequence=None):
    result = [str(exe), role, transport, str(number), key, path, method, str(timeout), family, payload.hex()]
    return result + ([str(sequence)] if sequence is not None else [])


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
                    if self.mode == "corrupt-request" and incoming:
                        if not data:
                            raise RuntimeError("cannot corrupt an empty datagram")
                        action = "corrupt"
                    elif self.mode == "blackhole":
                        action = "drop"
                    elif self.mode == "drop-reply" and not incoming and not dropped:
                        action, dropped = "drop", True
                    elif self.mode == "duplicate-request" and incoming and not duplicated:
                        action, duplicated = "duplicate", True
                    if len(self.trace) >= (8192 if self.mode == "dtls-reconnect" else 256):
                        raise RuntimeError("proxy trace limit exceeded")
                    forwarded = data[:-1] + bytes([data[-1] ^ 0x80]) if action == "corrupt" else data
                    self.trace.append({"direction": "request" if incoming else "response",
                                       "action": action, "hex": data.hex(), "forwarded_hex": forwarded.hex()})
                    if action != "drop":
                        target = self.back if incoming else self.front
                        address = self.dest if incoming else client
                        target.sendto(forwarded, address)
                        if action == "duplicate":
                            target.sendto(forwarded, address)
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


def replay_envelope(data):
    if len(data) < 5 or data[0] >> 6 != 1 or not 1 <= data[0] & 15 <= 8 or len(data) <= 4 + (data[0] & 15):
        raise AssertionError("missing token-bearing protected request bytes")
    replay = bytearray(data)
    replay[2] ^= 0x40
    replay[4] ^= 0x80
    return bytes(replay)


def oscore_fault_workflow(client, server):
    with Server(server, "oscore") as service:
        with Proxy(service.number, "dtls-reconnect") as relay:
            accepted = request(client, "oscore", relay.number, sequence=0, path="counter", method="POST")
            expect(accepted, 68, b"")
            expect(request(client, "oscore", service.number, sequence=10, path="counter"), 69, b"1")
            requests = [bytes.fromhex(row["hex"]) for row in relay.trace if row["direction"] == "request"]
            # Echo may cause one earlier challenged request. Replay the last
            # application request that actually produced the accepted POST.
            captured = next(wire for wire in reversed(requests) if len(wire) > 1 and wire[1] != 0)
            # RFC 8613 leaves outer MID/Token outside integrity protection.
            # Change both; preserve every option/ciphertext byte. Keep the
            # first relay bound so a new socket cannot reuse its source port.
            replay = replay_envelope(captured)
            accepted_source = relay.back.getsockname()
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
                sock.bind(("127.0.0.1", 0))
                replay_source = sock.getsockname()
                if replay_source == accepted_source:
                    raise AssertionError("replay did not change source endpoint")
                sock.settimeout(.5)
                if sock.sendto(replay, ("127.0.0.1", service.number)) != len(replay):
                    raise AssertionError("replay datagram was not completely sent")
                try:
                    replay_reply = sock.recvfrom(4096)[0].hex()
                except socket.timeout:
                    replay_reply = None
        expect(request(client, "oscore", service.number, sequence=20, path="counter"), 69, b"1")
        # A forged far-future request cannot advance the recipient window.
        with Proxy(service.number, "corrupt-request") as corrupted:
            refused = request(client, "oscore", corrupted.number, sequence=1000,
                              path="counter", method="POST", timeout=500)
        expect_refusal(refused)
        if not any(row["action"] == "corrupt" and row["hex"] != row["forwarded_hex"] for row in corrupted.trace):
            raise AssertionError("authenticated corruption was not exercised")
        expect(request(client, "oscore", service.number, sequence=30, path="counter"), 69, b"1")
        expect(request(client, "oscore", service.number, sequence=1000, path="counter", method="POST"), 68, b"")
        final = request(client, "oscore", service.number, sequence=1100, path="counter")
        expect(final, 69, b"2")
        return {"accepted_trace": relay.trace, "accepted_source": accepted_source, "replay_source": replay_source, "replayed_hex": replay.hex(), "replay_reply_hex": replay_reply,
                "corrupted_trace": corrupted.trace, "refused": refused, "final": final,
                "verified": ["replayed ciphertext cannot repeat POST effect", "bad-tag future request cannot consume replay window", "valid sequence remains usable after refusal"],
                "scope": "IPv4 UDP public C.1 context; sequential bounded counter fixture; not persistent or concurrent security qualification"}


def oscore_block2_workflow(client, server):
    evidence = []
    for mode in ("dtls-reconnect", "drop-reply", "duplicate-request"):
        with Server(server, "oscore") as service:
            # Complete any B.1.2 Echo challenge before the faulted transfer.
            expect(request(client, "oscore", service.number, sequence=0))
            with Proxy(service.number, mode) as relay:
                result = request(client, "oscore", relay.number, sequence=100,
                                 path="large", timeout=6500)
            expect(result, 69, LARGE)
            requests = [row for row in relay.trace if row["direction"] == "request"]
            if len(requests) < 2:
                raise AssertionError("large transfer did not exercise follow-up datagrams")
            if mode != "dtls-reconnect" and not any(row["action"] != "forward" for row in relay.trace):
                raise AssertionError("requested transfer fault was not exercised")
            refused = request(client, "oscore", service.number, sequence=1000,
                              path="large", key="incorrect", timeout=500)
            expect_refusal(refused)
            recovered = request(client, "oscore", service.number, sequence=1100, path="large")
            expect(recovered, 69, LARGE)
            evidence.append({"mode": mode, "result": result, "trace": relay.trace,
                             "wrong_key_refusal": refused, "recovered": recovered})
    return {"transfers": evidence, "expected_length": len(LARGE),
            "expected_sha256": hashlib.sha256(LARGE).hexdigest(),
            "scope": "Exact 2000-byte protected Block2 GET on IPv4; first reply loss and request duplication after Echo warm-up. No Block1, Observe, Q-Block, arbitrary reordering or persistent-context claim."}


def upload_workflow(client, server, transport="udp", family="ipv4"):
    """Exact Block1 bodies and one wrong byte, across one fresh server process."""
    changed = bytearray(LARGE)
    changed[-1] ^= 1

    def direct(**kwargs):
        if family != "ipv6":
            return request(client, transport, service.number, family=family, path="upload",
                           timeout=6500, **kwargs)
        with Proxy(service.number, "dtls-reconnect", family=family) as relay:
            result = request(client, transport, relay.number, family=family, path="upload",
                             timeout=6500, **kwargs)
        if not any(row["direction"] == "request" for row in relay.trace):
            raise AssertionError("no IPv6 request datagrams")
        return result

    def traced(payload):
        with Proxy(service.number, "dtls-reconnect", family=family) as relay:
            result = request(client, transport, relay.number, family=family, path="upload",
                             method="POST", payload=payload, timeout=6500)
            count = sum(row["direction"] == "request" for row in relay.trace)
        return result, count

    with Server(server, transport, family=family) as service:
        created, first_requests = traced(LARGE)
        expect(created, 65, b"")
        if first_requests < 2:
            raise AssertionError(f"2000-byte upload used {first_requests} request datagrams")
        expect(direct(), 69, b"1:1")
        refused = direct(method="POST", payload=bytes(changed))
        expect(refused, 128, b"")
        expect(direct(), 69, b"1:2")
        wide, wide_requests = traced(UPLOAD_4K)
        expect(wide, 65, b"")
        if wide_requests < 2:
            raise AssertionError(f"4096-byte upload used {wide_requests} request datagrams")
        readback = direct()
        expect(readback, 69, b"2:3")
        return {"transport": transport, "family": family,
                "lengths": [len(LARGE), len(UPLOAD_4K)],
                "sha256": {"2000": hashlib.sha256(LARGE).hexdigest(),
                           "4096": hashlib.sha256(UPLOAD_4K).hexdigest()},
                "request_datagrams": {"2000": first_requests, "4096": wide_requests},
                "wrong_byte": refused, "readback": readback,
                "verified": ["exact 2000-byte Block1 POST creates once",
                             "one wrong byte is 4.00 and does not increment accepted",
                             "exact 4096-byte Block1 POST creates once"],
                "unqualified": ["missing or duplicate blocks", "body-limit negotiation", "OSCORE"]
                + ([] if transport == "dtls" and family == "ipv6" else ["IPv6 DTLS"])}


CONTINUE = 95
INCOMPLETE = 136
TOO_LARGE = 141


def coap_option(delta, value):
    """One CoAP option. Delta and length use the RFC 7252 extended nibble form."""
    def field(n):
        if n < 13:
            return n, b""
        if n < 269:
            return 13, bytes([n - 13])
        if n < 65805:
            return 14, (n - 269).to_bytes(2, "big")
        raise ValueError("CoAP option field does not fit")
    delta_nibble, delta_ext = field(delta)
    length_nibble, length_ext = field(len(value))
    return bytes([(delta_nibble << 4) | length_nibble]) + delta_ext + length_ext + value


def coap_options(items):
    encoded = bytearray()
    previous = 0
    for number, value in items:
        if number < previous:
            raise ValueError("CoAP options are not in ascending order")
        encoded += coap_option(number - previous, value)
        previous = number
    return bytes(encoded)


def block1_value(num, more, szx):
    if not 0 <= num < 16 or not 0 <= szx <= 6:
        raise ValueError("Block1 value is outside the one-byte encoding")
    return bytes([(num << 4) | ((1 if more else 0) << 3) | szx])


def coap_message(code, mid, token, options, payload=b"", *, non=False):
    if not 0 < len(token) <= 8 or not 0 <= mid <= 0xFFFF or not 0 <= code <= 0xFF:
        raise ValueError("invalid CoAP header field")
    message = bytes([(0x50 if non else 0x40) | len(token), code, mid >> 8, mid & 0xFF]) + token + coap_options(options)
    if payload:
        message += b"\xff" + payload
    return message


def coap_payload(packet):
    if len(packet) < 4 or packet[0] >> 6 != 1 or (packet[0] & 15) > 8:
        raise AssertionError("not a CoAP datagram")
    index = 4 + (packet[0] & 15)
    while index < len(packet):
        if packet[index] == 0xFF:
            return packet[index + 1:]
        delta_nibble = packet[index] >> 4
        length_nibble = packet[index] & 15
        index += 1
        if delta_nibble == 15 or length_nibble == 15:
            raise AssertionError("malformed CoAP option")
        if delta_nibble == 13:
            delta_nibble = packet[index] + 13
            index += 1
        elif delta_nibble == 14:
            delta_nibble = int.from_bytes(packet[index:index + 2], "big") + 269
            index += 2
        if length_nibble == 13:
            length_nibble = packet[index] + 13
            index += 1
        elif length_nibble == 14:
            length_nibble = int.from_bytes(packet[index:index + 2], "big") + 269
            index += 2
        index += length_nibble
    return b""


def coap_roundtrip(sock, address, packet, timeout=2):
    """Send from one bound socket so Block1 identity stays on that endpoint."""
    sock.settimeout(timeout)
    if sock.sendto(packet, address) != len(packet):
        raise AssertionError("CoAP probe was not completely sent")
    try:
        data, sender = sock.recvfrom(4096)
    except (socket.timeout, ConnectionResetError) as error:
        raise AssertionError(f"no CoAP reply: {error}") from error
    if (sender[0], sender[1]) != address or ((data[0] >> 4) & 3) != 2:
        raise AssertionError(f"unexpected CoAP acknowledgement from {sender}")
    return data


def coap_exchange(sock, address, packet, timeout=2):
    data = coap_roundtrip(sock, address, packet, timeout)
    return data[1], coap_payload(data)


def decoded_options(packet):
    if len(packet) < 4 or packet[0] >> 6 != 1 or (packet[0] & 15) > 8:
        raise AssertionError("not a CoAP datagram")
    index = 4 + (packet[0] & 15)
    number = 0
    found = {}
    while index < len(packet) and packet[index] != 0xFF:
        delta_nibble = packet[index] >> 4
        length_nibble = packet[index] & 15
        index += 1
        if delta_nibble == 15 or length_nibble == 15 or index > len(packet):
            raise AssertionError("malformed CoAP option")
        delta, length = delta_nibble, length_nibble
        if delta_nibble == 13:
            delta = packet[index] + 13
            index += 1
        elif delta_nibble == 14:
            delta = int.from_bytes(packet[index:index + 2], "big") + 269
            index += 2
        if length_nibble == 13:
            length = packet[index] + 13
            index += 1
        elif length_nibble == 14:
            length = int.from_bytes(packet[index:index + 2], "big") + 269
            index += 2
        if index + length > len(packet):
            raise AssertionError("CoAP option exceeds the datagram")
        number += delta
        found.setdefault(number, []).append(packet[index:index + length])
        index += length
    return found


def block1_fields(value):
    if len(value) not in (1, 2, 3):
        raise AssertionError(f"Block1 length {len(value)} is outside 1..3")
    raw = int.from_bytes(value, "big")
    return raw >> 4, bool(raw & 8), raw & 7


def upload_block(mid, token, num, more, szx, payload, size1=None):
    options = [(11, b"upload"), (12, bytes([42])), (27, block1_value(num, more, szx))]
    if size1 is not None:
        encoded = size1.to_bytes(4, "big").lstrip(b"\x00") or b"\x00"
        options.append((60, encoded))
    return coap_message(2, mid, token, options, payload)


def block1_fault_workflow(server):
    """Independent Block1 refusals. The handler must stay cold."""
    pattern = bytes(i % 251 for i in range(1024))
    small = pattern[:64]

    def session(check):
        with Server(server, "udp") as service:
            address = ("127.0.0.1", service.number)
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            sock.bind(("127.0.0.1", 0))
            state = {"token": 1, "mid": 1}

            def fresh(token=None):
                state["mid"] = state["mid"] % 0xFFFF + 1
                if token is None:
                    state["token"] = state["token"] % 255 + 1
                    token = bytes([state["token"]])
                return state["mid"], token

            def post(token, num, more, szx, payload, size1=None):
                mid, _ = fresh(token)
                code, _ = coap_exchange(
                    sock, address, upload_block(mid, token, num, more, szx, payload, size1))
                return code

            def counts():
                mid, token = fresh()
                code, body = coap_exchange(
                    sock, address, coap_message(1, mid, token, [(11, b"upload")]))
                return code, body

            try:
                return check(post, counts)
            finally:
                sock.close()

    def ordered(post, counts):
        token = bytes([9])
        if post(token, 0, True, 2, small, size1=1) != CONTINUE:
            raise AssertionError("Size1 did not leave an unfinished block at 2.31")
        if counts() != (69, b"0:0"):
            raise AssertionError("Size1 completed or invoked the upload handler")
        if post(token, 0, True, 2, small) != CONTINUE:
            raise AssertionError("duplicate Block1 NUM was not replayed as 2.31")
        if post(token, 1, True, 2, small) != CONTINUE:
            raise AssertionError("duplicate Block1 NUM reset the transfer")
        if post(token, 3, True, 2, small) != INCOMPLETE:
            raise AssertionError("Block1 gap was not 4.08")
        if counts() != (69, b"0:0"):
            raise AssertionError("gap or duplicate invoked the upload handler")
        mismatched = bytes([10])
        if post(mismatched, 0, True, 2, small) != CONTINUE:
            raise AssertionError("initial SZX was refused")
        if post(mismatched, 1, True, 6, pattern) != INCOMPLETE:
            raise AssertionError("changed SZX was not 4.08")
        return counts()

    def overflow(post, counts):
        token = bytes([11])
        for num in range(4):
            if post(token, num, True, 6, pattern) != CONTINUE:
                raise AssertionError(f"block {num} of a 4096-byte body was refused early")
        if post(token, 4, True, 6, pattern) != TOO_LARGE:
            raise AssertionError("body past 4096 bytes was not 4.13")
        final = counts()
        if final != (69, b"0:0"):
            raise AssertionError(f"upload handler ran during refusal: {final}")
        return final

    session(ordered)
    final = session(overflow)
    return {"codes": {"continue": CONTINUE, "incomplete": INCOMPLETE, "too_large": TOO_LARGE},
            "body_limit": 4096, "final": {"code": final[0], "payload": final[1].decode()},
            "verified": ["Size1 does not complete M=1", "duplicate NUM stays 2.31 and preserves progress",
                         "gap is 4.08", "unscaled larger SZX is 4.08", "a block past 4096 bytes is 4.13",
                         "the upload handler stays at 0:0"],
            "unqualified": ["OSCORE block faults", "client SZX reduction", "Q-Block"]}


def smaller_block_upload(server):
    """Client starts at 256-byte blocks and keeps that size through completion."""
    szx = 4
    size = 1 << (szx + 4)
    body = LARGE
    with Server(server, "udp") as service:
        address = ("127.0.0.1", service.number)
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        try:
            offset = num = 0
            mid = 1
            while offset < len(body):
                chunk = body[offset:offset + size]
                more = offset + len(chunk) < len(body)
                mid += 1
                reply = coap_roundtrip(
                    sock, address, upload_block(mid, b"\x21", num, more, szx, chunk))
                echoed = decoded_options(reply).get(27, [])
                if more:
                    if len(echoed) != 1:
                        raise AssertionError(f"block {num} did not echo Block1: {echoed!r}")
                    echoed_num, _, echoed_szx = block1_fields(echoed[0])
                    if (echoed_num, echoed_szx) != (num, szx):
                        raise AssertionError(f"block {num} was not kept at SZX {szx}: {echoed!r}")
                elif len(echoed) == 1:
                    echoed_num, _, echoed_szx = block1_fields(echoed[0])
                    if (echoed_num, echoed_szx) != (num, szx):
                        raise AssertionError(f"final block echo changed size: {echoed!r}")
                elif echoed:
                    raise AssertionError(f"final block had repeated Block1: {echoed!r}")
                if reply[1] != (CONTINUE if more else 65):
                    raise AssertionError(f"block {num} returned {reply[1]}")
                offset += len(chunk)
                num += 1
            if num < 3:
                raise AssertionError(f"256-byte blocks did not split the body: {num}")
            code, counts = coap_exchange(
                sock, address, coap_message(1, mid + 1, b"\x22", [(11, b"upload")]))
        finally:
            sock.close()
    if (code, counts) != (69, b"1:1"):
        raise AssertionError(f"smaller blocks did not create the body once: {code} {counts!r}")
    return {"szx": szx, "block_bytes": size, "blocks": num, "length": len(body),
            "readback": counts.decode(),
            "verified": ["client-selected 256-byte Block1 completes the exact 2000-byte body once",
                         "each non-final acknowledgement echoes that SZX and NUM"],
            "unqualified": ["server-requested downshift", "Q-Block"]}


def scaled_smaller_block(server):
    """RFC 7959 Figure 9 against one server: 64-byte block 0, then 16-byte block 4."""
    first = bytes(i % 251 for i in range(64))
    tail = bytes((64 + i) % 251 for i in range(16))
    with Server(server, "udp") as service:
        address = ("127.0.0.1", service.number)
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        try:
            opened = coap_roundtrip(sock, address, upload_block(2, b"\x31", 0, True, 2, first))
            if opened[1] != CONTINUE:
                raise AssertionError(f"first 64-byte block returned {opened[1]}")
            skipped = coap_roundtrip(sock, address, upload_block(3, b"\x31", 1, True, 0, tail))
            if skipped[1] != INCOMPLETE:
                raise AssertionError(f"unaligned smaller block returned {skipped[1]}")
            code, counts = coap_exchange(
                sock, address, coap_message(1, 5, b"\x32", [(11, b"upload")]))
            if (code, counts) != (69, b"0:0"):
                raise AssertionError(f"unaligned block reached the handler: {code} {counts!r}")
            finished = coap_roundtrip(sock, address, upload_block(4, b"\x31", 4, False, 0, tail))
            if finished[1] != 128:
                raise AssertionError(f"scaled block did not reach the length check: {finished[1]}")
            code, counts = coap_exchange(
                sock, address, coap_message(1, 6, b"\x33", [(11, b"upload")]))
        finally:
            sock.close()
    if (code, counts) != (69, b"0:1"):
        raise AssertionError(f"scaled body was not delivered once: {code} {counts!r}")
    return {"first_bytes": 64, "next_num": 4, "next_bytes": 16, "readback": counts.decode(),
            "verified": ["64-byte block 0 continues", "16-byte block 1 is 4.08",
                         "16-byte block 4 is delivered once"],
            "unqualified": ["Q-Block size change", "peer servers other than Coaptic"]}


def qblock1_message(mid, token, num, more, szx, payload, size1, tag, non=False):
    encoded = size1.to_bytes(4, "big").lstrip(b"\x00") or b"\x00"
    options = [(11, b"upload"), (12, bytes([42])), (19, block1_value(num, more, szx)), (60, encoded)]
    if tag is not None:
        options.append((292, tag))
    return coap_message(2, mid, token, options, payload, non=non)


def await_datagram(sock, address, timeout):
    sock.settimeout(timeout)
    try:
        data, sender = sock.recvfrom(4096)
    except (socket.timeout, ConnectionResetError) as error:
        raise AssertionError(f"no CoAP reply: {error}") from error
    if (sender[0], sender[1]) != address or data[0] >> 6 != 1:
        raise AssertionError(f"unexpected datagram from {sender}")
    return data


def qblock1_upload(server):
    """One Coaptic Q-Block1 upload. Recovery and other peers stay out of this proof."""
    body = LARGE
    first, rest = body[:1024], body[1024:]
    with Server(server, "udp") as service:
        address = ("127.0.0.1", service.number)
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        try:
            def post(mid, token, num, more, szx, payload, size1, tag):
                return coap_roundtrip(
                    sock, address, qblock1_message(mid, token, num, more, szx, payload, size1, tag))

            def counts(mid, token):
                return coap_exchange(
                    sock, address, coap_message(1, mid, token, [(11, b"upload")]))

            missing = post(2, b"\x41", 0, True, 6, first, len(body), None)
            if missing[1] != 128:
                raise AssertionError(f"missing Request-Tag returned {missing[1]}")
            if counts(3, b"\x51") != (69, b"0:0"):
                raise AssertionError("missing Request-Tag reached the upload handler")
            opened = post(4, b"\x42", 0, True, 6, first, len(body), b"a")
            if opened[1] != 0 or (opened[0] & 15) != 0 or len(opened) != 4:
                raise AssertionError(f"incomplete Q-Block1 was not an empty ACK: {opened!r}")
            changed = post(5, b"\x42", 1, False, 2, rest[:64], len(body), b"a")
            if changed[1] != INCOMPLETE:
                raise AssertionError(f"changed Q-Block1 size returned {changed[1]}")
            if counts(6, b"\x52") != (69, b"0:0"):
                raise AssertionError("changed Q-Block1 size reached the upload handler")
            if post(7, b"\x43", 0, True, 6, first, len(body), b"b")[1] != 0:
                raise AssertionError("exact Q-Block1 payload 0 was not acknowledged")
            created = post(8, b"\x43", 1, False, 6, rest, len(body), b"b")
            if created[1] != 65:
                raise AssertionError(f"exact Q-Block1 body returned {created[1]}")
            code, readback = counts(9, b"\x53")
        finally:
            sock.close()
    if (code, readback) != (69, b"1:1"):
        raise AssertionError(f"Q-Block1 body was not created once: {code} {readback!r}")
    return {"blocks": 2, "block_bytes": 1024, "length": len(body), "readback": readback.decode(),
            "verified": ["missing Request-Tag is 4.00", "incomplete confirmable payload is an empty ACK",
                         "changed SZX is 4.08", "exact 2000-byte body is created once"],
            "unqualified": ["Q-Block recovery", "peer servers other than Coaptic"]}


def qblock1_missing(server):
    """A skipped NON Q-Block1 number is reported, then that payload is delivered once."""
    token = b"\x61"
    first = bytes(i % 251 for i in range(16))
    middle = bytes((16 + i) % 251 for i in range(16))
    tail = bytes((32 + i) % 251 for i in range(8))
    with Server(server, "udp") as service:
        address = ("127.0.0.1", service.number)
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        try:
            def send(mid, num, more, payload):
                packet = qblock1_message(mid, token, num, more, 0, payload, 40, b"h", non=True)
                if sock.sendto(packet, address) != len(packet):
                    raise AssertionError("Q-Block1 probe was not completely sent")

            send(2, 0, True, first)
            send(3, 2, False, tail)
            report = await_datagram(sock, address, 6)
            if ((report[0] >> 4) & 3) != 1 or report[1] != INCOMPLETE or (report[0] & 15) != 1:
                raise AssertionError(f"hole was not a NON 4.08: {report!r}")
            if report[4:5] != token:
                raise AssertionError(f"4.08 token was not the request token: {report!r}")
            if decoded_options(report).get(12) != [bytes([1, 16])]:
                raise AssertionError("4.08 content format was not missing-blocks")
            if coap_payload(report) != b"\x01":
                raise AssertionError(f"4.08 did not name block 1: {coap_payload(report)!r}")
            if coap_exchange(sock, address, coap_message(1, 4, b"\x62", [(11, b"upload")])) != (69, b"0:0"):
                raise AssertionError("missing Q-Block1 payload reached the handler")
            filled = coap_roundtrip(
                sock, address, qblock1_message(5, token, 1, True, 0, middle, 40, b"h"))
            if filled[1] != 128:
                raise AssertionError(f"filled hole returned {filled[1]}")
            code, readback = coap_exchange(
                sock, address, coap_message(1, 6, b"\x63", [(11, b"upload")]))
        finally:
            sock.close()
    if (code, readback) != (69, b"0:1"):
        raise AssertionError(f"filled hole was not delivered once: {code} {readback!r}")
    return {"missing": 1, "reported_format": 272, "length": 40, "readback": readback.decode(),
            "verified": ["NON 4.08 names block 1", "handler stays at 0:0 until that payload arrives",
                         "the reported payload is delivered once"],
            "unqualified": ["full-window continuation", "peer servers other than Coaptic", "protected Q-Block"]}


def oscore_upload_workflow(client, server):
    """Exact protected uploads. Each fresh client process has its own sender sequence."""
    changed = bytearray(LARGE)
    changed[-1] ^= 1
    with Server(server, "oscore") as service:
        expect(request(client, "oscore", service.number, sequence=0))
        created = request(client, "oscore", service.number, sequence=100, path="upload",
                          method="POST", payload=LARGE, timeout=6500)
        expect(created, 65, b"")
        expect(request(client, "oscore", service.number, sequence=200, path="upload"), 69, b"1:1")
        expect(request(client, "oscore", service.number, sequence=300, path="upload", method="POST",
                       payload=bytes(changed), timeout=6500), 128, b"")
        expect(request(client, "oscore", service.number, sequence=400, path="upload"), 69, b"1:2")
        expect(request(client, "oscore", service.number, sequence=500, path="upload", method="POST",
                       payload=UPLOAD_4K, timeout=6500), 65, b"")
        readback = request(client, "oscore", service.number, sequence=700, path="upload")
        expect(readback, 69, b"2:3")
        refused = request(client, "oscore", service.number, sequence=1000, path="upload",
                          method="POST", payload=LARGE, key="incorrect", timeout=500)
        expect_refusal(refused)
        preserved = request(client, "oscore", service.number, sequence=1100, path="upload")
        expect(preserved, 69, b"2:3")
        return {"sha256": {"2000": hashlib.sha256(LARGE).hexdigest(),
                           "4096": hashlib.sha256(UPLOAD_4K).hexdigest()},
                "wrong_key": refused, "readback": readback,
                "verified": ["exact protected 2000- and 4096-byte Block1 creates",
                             "wrong byte does not increment accepted",
                             "wrong key cannot change the accepted count"],
                "unqualified": ["protected block loss or duplication", "OSCORE client SZX negotiation"]}


def oscore_upload_fault_workflow(client, server):
    """Lost first reply and duplicated request during one protected Block1 upload."""
    evidence = []
    for mode in ("dtls-reconnect", "drop-reply", "duplicate-request"):
        with Server(server, "oscore") as service:
            expect(request(client, "oscore", service.number, sequence=0))
            with Proxy(service.number, mode) as relay:
                created = request(client, "oscore", relay.number, sequence=100, path="upload",
                                  method="POST", payload=LARGE, timeout=6500)
            expect(created, 65, b"")
            requests = [row for row in relay.trace if row["direction"] == "request"]
            if len(requests) < 2:
                raise AssertionError("protected upload did not exercise follow-up datagrams")
            if mode != "dtls-reconnect" and not any(row["action"] != "forward" for row in relay.trace):
                raise AssertionError("requested upload fault was not exercised")
            expect(request(client, "oscore", service.number, sequence=200, path="upload"), 69, b"1:1")
            refused = request(client, "oscore", service.number, sequence=1000, path="upload",
                              method="POST", payload=LARGE, key="incorrect", timeout=500)
            expect_refusal(refused)
            preserved = request(client, "oscore", service.number, sequence=1100, path="upload")
            expect(preserved, 69, b"1:1")
            evidence.append({"mode": mode, "created": created, "trace": relay.trace,
                             "wrong_key_refusal": refused, "preserved": preserved})
    return {"expected_length": len(LARGE), "expected_sha256": hashlib.sha256(LARGE).hexdigest(),
            "transfers": evidence,
            "verified": ["exact protected upload after a lost first reply",
                         "exact protected upload after a duplicated request",
                         "one handler effect", "wrong key cannot add another effect"],
            "unqualified": ["IPv6", "DTLS", "Q-Block", "client SZX reduction"]}


def ipv6_dtls_request(client, number, traces, **kwargs):
    # Each session has a fresh server-visible endpoint; IPv6-only sockets
    # prevent a fixture silently falling back to IPv4 from satisfying the case.
    with Proxy(number, "dtls-reconnect", family="ipv6") as relay:
        result = request(client, "dtls", relay.number, family="ipv6", **kwargs)
    if not any(row["direction"] == "request" for row in relay.trace):
        raise AssertionError("no IPv6 request datagrams")
    traces.append(relay.trace)
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
    evidence, traces = [], []
    with Server(server, transport, family=family) as service:
        probe = ipv6_probe(service.number) if family == "ipv6" and transport == "udp" else None
        def exchange(**kwargs):
            if family == "ipv6" and transport == "dtls":
                return ipv6_dtls_request(client, service.number, traces, **kwargs)
            return request(client, transport, service.number, family=family, **kwargs)
        refusal = None
        for index, (method, payload, code, body) in enumerate(steps):
            if transport == "dtls" and index == 2:
                # A refused replacement must leave the accepted PUT intact.
                refusal = exchange(path="methods",
                                  method="PUT", payload=b"poison", key="incorrect", timeout=1500)
                expect_refusal(refusal, handshake=True)
            result = exchange(path="methods", method=method, payload=payload)
            expect(result, code, body)
            evidence.append({"method": method, "request_hex": payload.hex(), "expected_code": code,
                             "expected_payload_hex": None if body is None else body.hex(), "response": result})
    return {"steps": evidence, "transport": transport, "address_family": family,
            "ipv6_socket_probe": probe, "ipv6_dtls_traces": traces, "wrong_key_result": refusal, "state_limit_bytes": 64, "patch_format": "fixture octet-stream: PATCH +suffix; IPATCH =replacement"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--libcoap-oscore-unavailable", action="store_true", help="Explicitly exclude C-peer OSCORE for this build")
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
              "libcoap_source": "851533c3cf63d16984d370ce39d586ecb3694971",
              "limitations": ["Only named plaintext/PSK DTLS/OSCORE assertions; Observe, certificates and full ETSI coverage remain unqualified"],
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

    for client, server in [("coaptic", "coaptic"), ("coaptic", "libcoap"), ("libcoap", "coaptic")]:
        if (args.libcoap_udp_only or args.libcoap_oscore_unavailable) and "libcoap" in (client, server):
            continue
        def oscore_state(client=client, server=server):
            with Server(peers[server], "oscore") as service:
                traces, results = [], []
                sequence = 0
                def exchange(**kwargs):
                    nonlocal sequence
                    with Proxy(service.number, "dtls-reconnect") as relay:
                        result = request(peers[client], "oscore", relay.number, sequence=sequence, **kwargs)
                    sequence += 100
                    if not relay.trace:
                        raise AssertionError("OSCORE exchange had no wire evidence")
                    traces.append(relay.trace)
                    results.append(result)
                    return result
                initial = exchange()
                expect(initial)
                if client == "coaptic" and server == "libcoap" and initial.get("echo_retries") != 1:
                    raise AssertionError("libcoap authenticated Echo challenge was not exercised exactly once")
                expect(exchange(path="methods", method="PUT", payload=b"alpha"), 65, b"")
                refused = exchange(path="methods", method="PUT", payload=b"poison", key="incorrect", timeout=1500)
                expect_refusal(refused)
                expect(exchange(path="methods"), 69, b"alpha")
                plain = request(peers[client], "udp", service.number, path="methods", method="PUT", payload=b"poison")
                expect(plain, 129, None)
                expect(exchange(path="methods"), 69, b"alpha")
                return {"server": service.ready, "traces": traces, "results": results, "plaintext_refusal": plain,
                        "fixture_context": "RFC 8613 C.1 public keys; client starting sequences 0,100,200,300,400; one Echo retry may consume the next sequence",
                        "verified": ["exact authenticated GET", "PUT/readback", "wrong-key and plaintext state preservation"],
                        "unqualified": ["persistent keys/sequences", "replay/corruption campaign", "OSCORE Observe/block transfer"]}
        case(f"oscore-state:{client}->{server}", oscore_state)

    for client, server in [("coaptic", "coaptic"), ("coaptic", "libcoap"), ("libcoap", "coaptic")]:
        if (args.libcoap_udp_only or args.libcoap_oscore_unavailable) and "libcoap" in (client, server):
            continue
        case(f"oscore-faults:{client}->{server}",
             lambda client=client, server=server: oscore_fault_workflow(peers[client], peers[server]))
        case(f"oscore-block2:{client}->{server}",
             lambda client=client, server=server: oscore_block2_workflow(peers[client], peers[server]))

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
                                     ("udp", "ipv6", "methods-ipv6-udp"),
                                     ("dtls", "ipv6", "methods-ipv6-dtls")):
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
                traces = []
                def exchange(**kwargs):
                    return ipv6_dtls_request(peers[client], service.number, traces, **kwargs)
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

    for transport, family, label in (("udp", "ipv4", "upload-udp"),
                                     ("dtls", "ipv4", "upload-dtls"),
                                     ("udp", "ipv6", "upload-ipv6-udp"),
                                     ("dtls", "ipv6", "upload-ipv6-dtls")):
        for client, server in [("coaptic", "coaptic"), ("coaptic", "coap-rs"), ("coap-rs", "coaptic"),
                               ("coaptic", "libcoap"), ("libcoap", "coaptic")]:
            if transport == "dtls" and args.libcoap_udp_only and "libcoap" in (client, server):
                continue
            case(f"{label}:{client}->{server}",
                 lambda client=client, server=server, transport=transport, family=family:
                 upload_workflow(peers[client], peers[server], transport, family))

    case("block1-faults:coaptic", lambda: block1_fault_workflow(peers["coaptic"]))
    case("block1-scaled:coaptic", lambda: scaled_smaller_block(peers["coaptic"]))
    case("qblock1-upload:coaptic", lambda: qblock1_upload(peers["coaptic"]))
    case("qblock1-missing:coaptic", lambda: qblock1_missing(peers["coaptic"]))
    for server_name in ("coaptic", "coap-rs", "libcoap"):
        case(f"block1-szx:{server_name}",
             lambda server_name=server_name: smaller_block_upload(peers[server_name]))

    for client, server in [("coaptic", "coaptic"), ("coaptic", "libcoap"), ("libcoap", "coaptic")]:
        if (args.libcoap_udp_only or args.libcoap_oscore_unavailable) and "libcoap" in (client, server):
            continue
        case(f"oscore-upload:{client}->{server}",
             lambda client=client, server=server: oscore_upload_workflow(peers[client], peers[server]))
        case(f"oscore-upload-faults:{client}->{server}",
             lambda client=client, server=server: oscore_upload_fault_workflow(peers[client], peers[server]))

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
        libcoap_dtls=not args.libcoap_udp_only, libcoap_oscore=not (args.libcoap_udp_only or args.libcoap_oscore_unavailable), system=platform.system().lower())
    report["passed"] = report["coverage"]["complete"]
    report["failures"] = len(report["coverage"]["problems"])
    for problem in report["coverage"]["problems"]:
        print(f"FAIL coverage: {problem}", flush=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
