import unittest
from fuzz import evidence


class FuzzEvidenceTests(unittest.TestCase):
    def test_feedback_and_executed_inputs_are_both_required(self):
        valid = "#100 DONE cov: 42 ft: 60\nstat::number_of_executed_units: 100\n"
        self.assertEqual(evidence(valid), {"coverage_points": 42, "features": 60, "executed_inputs": 100})
        for output in ["", "Done 100 runs", "cov: 42 ft: 60", "stat::number_of_executed_units: 100",
                       valid.replace("cov: 42", "cov: 0"), valid.replace("ft: 60", "ft: 0"),
                       valid.replace("units: 100", "units: 0")]:
            with self.assertRaises(ValueError):
                evidence(output)


if __name__ == "__main__":
    unittest.main()
