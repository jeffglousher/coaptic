"""Fail-closed host qualification accounting and retained failure evidence."""
import subprocess
import unittest
from host import CASES, run_case, run_matrix


class HostEvidenceTests(unittest.TestCase):
    def test_records_executed_and_ignored_counts(self):
        def execute(command, **kwargs):
            self.assertEqual(command[:5], ["cargo", "+1.97.1", "test", "--locked", "-p"])
            self.assertEqual(kwargs["timeout"], 900)
            return subprocess.CompletedProcess(command, 0,
                "test result: ok. 9 passed; 0 failed; 2 ignored;\ntest result: ok. 3 passed; 0 failed; 0 ignored;", "")
        result = run_case("core", ["--no-default-features"], execute)
        self.assertTrue(result["passed"])
        self.assertEqual((result["executed"], result["ignored"]), (12, 2))

    def test_zero_or_missing_tests_cannot_pass(self):
        for output in ("", "test result: ok. 0 passed; 0 failed; 8 ignored;"):
            result = run_case("core", [], lambda command, **kwargs: subprocess.CompletedProcess(command, 0, output, ""))
            self.assertFalse(result["passed"])

    def test_partial_success_does_not_hide_failure(self):
        for code, suffix in ((1, ""), (0, "test result: FAILED")):
            result = run_case("core", [], lambda command, **kwargs: subprocess.CompletedProcess(command, code,
                "test result: ok. 9 passed; 0 failed; 0 ignored;" + suffix, "retained diagnostic"))
            self.assertFalse(result["passed"])
            self.assertEqual(result["stderr"], "retained diagnostic")

    def test_timeout_retains_partial_output_and_continues_matrix(self):
        calls = []
        def execute(command, **kwargs):
            calls.append(command)
            if len(calls) == 2:
                raise subprocess.TimeoutExpired(command, 900, output=b"partial output", stderr=b"timeout diagnostic")
            return subprocess.CompletedProcess(command, 0, "test result: ok. 1 passed; 0 failed; 0 ignored;", "")
        results = run_matrix(execute)
        self.assertEqual(len(calls), len(CASES))
        self.assertFalse(results[1]["passed"])
        self.assertTrue(results[1]["timed_out"])
        self.assertEqual(results[1]["stdout"], "partial output")
        self.assertEqual(results[1]["stderr"], "timeout diagnostic")
        self.assertEqual(sum(result["passed"] for result in results), len(CASES) - 1)

    def test_missing_executable_is_a_recorded_failure(self):
        def execute(*args, **kwargs):
            raise FileNotFoundError("compiler unavailable")
        result = run_case("core", [], execute)
        self.assertFalse(result["passed"])
        self.assertIn("compiler unavailable", result["stderr"])


if __name__ == "__main__":
    unittest.main()
