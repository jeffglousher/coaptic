"""Exercise wire fault injection and fail-closed result validation."""
import socket
import unittest
from run import Proxy, decode, expect, summary, validate_timing, measure_requests


class RunnerTests(unittest.TestCase):
    def test_wrong_schema_is_not_accepted(self):
        with self.assertRaises(RuntimeError):
            decode('{"schema":"other","event":"response"}')

    def test_payload_mismatch_is_not_success(self):
        with self.assertRaises(AssertionError):
            expect({"exit_code":0,"code":69,"payload_hex":"00"})
        with self.assertRaises(AssertionError):
            expect({"exit_code":1,"code":69,"payload_hex":""})

    def test_nearest_rank_percentiles(self):
        self.assertEqual(summary([4,1,3,2])["p50"], 2)
        self.assertEqual(summary([4,1,3,2])["p99"], 4)

    def test_nanosecond_evidence_preserves_sub_microsecond_samples(self):
        event = {"elapsed_ns": 1234567, "clock": {"name": "CLOCK_MONOTONIC", "resolution_ns": 1}}
        self.assertEqual(validate_timing(event), 1234567)
        self.assertEqual(summary([1234567,1234568])["mean"], 1234567.5)
        event["clock"]["resolution_ns"] = None
        self.assertEqual(validate_timing(event), 1234567)

    def test_invalid_timing_cannot_pass(self):
        for elapsed in (None, True, -1, 1.5, float("nan"), float("inf"), 60_000_000_001):
            with self.subTest(elapsed=elapsed), self.assertRaises(RuntimeError):
                validate_timing({"elapsed_ns": elapsed, "clock": {"name":"test", "resolution_ns":1}})
        for clock in (None, {}, {"name":"test"}, {"name":"test", "resolution_ns":0},
                      {"name":"test", "resolution_ns":True}):
            with self.subTest(clock=clock), self.assertRaises(RuntimeError):
                validate_timing({"elapsed_ns": 123, "clock": clock})
        with self.assertRaises(RuntimeError):
            decode('{"schema":"coaptic-peer/1","event":"response","elapsed_us":1000}')

    def test_failed_run_retains_samples_without_retrying(self):
        events = iter([
            {"exit_code":0, "code":69, "payload_hex":b"core-test-payload".hex(),
             "elapsed_ns":1234567, "host_total_ns":2000001,
             "clock":{"name":"test", "resolution_ns":1}},
            {"exit_code":1, "event":"error", "message":"deadline has elapsed"},
        ])
        result = measure_requests(100, lambda: next(events))
        self.assertEqual(result["samples_ns"]["request"], [1234567])
        self.assertEqual(result["failures"], 1)
        self.assertEqual(result["sample_failures"][0]["sample_index"], 1)
        self.assertEqual(result["request_ns"]["n"], 1)
        self.assertNotIn("serial_host_requests_per_second", result)

    def test_first_sample_failure_is_not_an_empty_success(self):
        result = measure_requests(100, lambda: {"exit_code":1})
        self.assertEqual(result["failures"], 1)
        self.assertEqual(result["samples_ns"]["request"], [])
        self.assertNotIn("request_ns", result)

    def test_duplicate_and_drop_are_real_datagrams(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as server, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as client:
            server.bind(("127.0.0.1",0))
            server.settimeout(1)
            client.settimeout(1)
            with Proxy(server.getsockname()[1], "duplicate-request") as proxy:
                client.sendto(b"request",("127.0.0.1",proxy.number))
                one, address=server.recvfrom(100)
                two, again=server.recvfrom(100)
                self.assertEqual((one,address),(two,again))
                server.sendto(b"reply",address)
                self.assertEqual(client.recvfrom(100)[0], b"reply")
            self.assertEqual(proxy.trace[0]["action"],"duplicate")
            with Proxy(server.getsockname()[1], "drop-reply") as proxy:
                client.sendto(b"request",("127.0.0.1",proxy.number))
                _,address=server.recvfrom(100)
                server.sendto(b"drop",address)
                server.sendto(b"keep",address)
                self.assertEqual(client.recvfrom(100)[0],b"keep")
            self.assertEqual(sum(e["action"]=="drop" for e in proxy.trace),1)


if __name__ == "__main__":
    unittest.main()
