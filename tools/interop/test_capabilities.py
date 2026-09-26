"""Coverage accounting must reject omissions rather than inflate tested surface."""
import ast
import copy
import unittest
from pathlib import Path

from capabilities import load_manifest, evaluate, validate_manifest
from run import expect_refusal


class CapabilityTests(unittest.TestCase):
    def setUp(self):
        self.manifest, self.digest = load_manifest()
        self.outcomes = [{"name": row["id"], "passed": True, "evidence": {"oracle": "fixture"}}
                         for row in self.manifest["cases"]]

    def check(self, outcomes, enabled=True, system="linux"):
        return evaluate(self.manifest, outcomes, libcoap_dtls=enabled, libcoap_oscore=enabled, system=system)

    def test_full_and_explicitly_limited_inventory(self):
        full = self.check(self.outcomes)
        self.assertTrue(full["complete"])
        self.assertEqual(full["enabled_cases"], 102)
        excluded = {row["id"] for row in self.manifest["cases"] if row["requires"]}
        limited = self.check([row for row in self.outcomes if row["name"] not in excluded], False, "windows")
        self.assertTrue(limited["complete"])
        self.assertEqual(limited["enabled_cases"], 79)
        self.assertEqual(sum(row["status"] == "build-excluded" for row in limited["cases"]), 23)
        self.assertEqual(limited["unqualified"], full["unqualified"])
        self.assertEqual(len(self.digest), 64)

    def test_oscore_exclusion_is_separate_from_dtls(self):
        excluded = {row["id"] for row in self.manifest["cases"] if "libcoap-oscore" in row["requires"]}
        result = evaluate(self.manifest, [row for row in self.outcomes if row["name"] not in excluded],
                          libcoap_dtls=True, libcoap_oscore=False, system="linux")
        self.assertTrue(result["complete"])
        self.assertEqual(result["enabled_cases"], 94)
        self.assertFalse(evaluate(self.manifest, self.outcomes, libcoap_dtls=True,
                                  libcoap_oscore=False, system="linux")["complete"])

    def test_missing_empty_duplicate_and_undeclared_runs_fail(self):
        variants = [[], self.outcomes[1:], self.outcomes + [self.outcomes[0]],
                    self.outcomes + [{"name": "invented", "passed": True, "evidence": {"x": 1}}]]
        for rows in variants:
            with self.subTest(rows=len(rows)):
                result = self.check(rows)
                self.assertFalse(result["complete"])
                self.assertTrue(result["problems"])

    def test_disabled_execution_and_undeclared_platform_fail(self):
        self.assertFalse(self.check(self.outcomes, False)["complete"])
        self.assertFalse(self.check(self.outcomes, system="darwin")["complete"])

    def test_bad_results_and_absent_evidence_fail(self):
        for value in (False, None, 1, "true"):
            rows = copy.deepcopy(self.outcomes)
            rows[0]["passed"] = value
            self.assertFalse(self.check(rows)["complete"])
        for evidence in (None, {}, []):
            rows = copy.deepcopy(self.outcomes)
            rows[0]["evidence"] = evidence
            self.assertFalse(self.check(rows)["complete"])

    def test_manifest_rejects_silent_scope_mutations(self):
        mutations = [lambda m: m.update(cases=[]),
                     lambda m: m["cases"].append(m["cases"][0]),
                     lambda m: m["cases"][0].update(positive_proof=[]),
                     lambda m: m["cases"][0].update(requires=["libcoap-dtls"]),
                     lambda m: m["gaps"][0].update(status="passed"),
                     lambda m: m["gaps"][0].update(executable_cases=["fake"])]
        for mutate in mutations:
            manifest = copy.deepcopy(self.manifest)
            mutate(manifest)
            with self.assertRaises(ValueError):
                validate_manifest(manifest)

    def test_declared_executables_resolve_to_real_function_definitions(self):
        root = Path(__file__).resolve().parents[2]
        for case in self.manifest["cases"]:
            filename, function = case["executable"].split(":")
            tree = ast.parse((root / filename).read_text(encoding="utf-8"))
            names = set()
            def walk(node, prefix=""):
                if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    prefix = f"{prefix}.{node.name}" if prefix else node.name
                    names.add(prefix)
                for child in ast.iter_child_nodes(node):
                    walk(child, prefix)
            walk(tree)
            self.assertIn(function, names)

    def test_process_crash_or_setup_error_cannot_count_as_auth_refusal(self):
        for code in (0, 2, -9, 3221225477, True):
            with self.assertRaises(AssertionError):
                expect_refusal({"event": "error", "exit_code": code, "message": "handshake timeout"}, handshake=True)
        for message in ("invalid arguments", "unsupported transport", "connection reset"):
            with self.assertRaises(AssertionError):
                expect_refusal({"event": "error", "exit_code": 1, "message": message}, handshake=True)
        for message in ("handshake failed", "decrypt error", "alert received", "deadline elapsed"):
            expect_refusal({"event": "error", "exit_code": 1, "message": message}, handshake=True)
        expect_refusal({"event": "error", "exit_code": 1, "message": "request timed out"})
        with self.assertRaises(AssertionError):
            expect_refusal({"event": "response", "exit_code": 1, "message": "timeout"})


if __name__ == "__main__":
    unittest.main()
