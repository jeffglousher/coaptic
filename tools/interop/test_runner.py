"""Exercise wire fault injection and fail-closed result validation."""
import socket
import unittest
from run import Proxy, decode, expect, summary


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
