import json
import unittest
from esp32 import read_capture


class DeviceEvidenceTests(unittest.TestCase):
    def fixture(self):
        case = {"run_id": "fresh-image", "oscore": True}
        result = {"schema": "coaptic-device/1", "chip": "esp32c3", "run_id": "fresh-image",
                  "oscore": True, "passed": True, "stack_high_water_bytes": 32000, "stack_capacity_bytes": 200000}
        return case, result

    def test_capture_must_match_image_and_nonempty_stack_measurement(self):
        case, result = self.fixture()
        raw = "boot log\nCOAPTIC_DEVICE " + json.dumps(result)
        self.assertEqual(read_capture(raw, case), result)
        for text in ["", raw + "\n" + raw, raw + "\nCOAPTIC_DEVICE_FAIL panic"]:
            with self.assertRaises(ValueError):
                read_capture(text, case)
        for key, value in [("run_id", "stale"), ("chip", "esp32"), ("oscore", False),
                           ("passed", False), ("stack_high_water_bytes", 0),
                           ("stack_high_water_bytes", 200000), ("stack_capacity_bytes", True)]:
            invalid = dict(result, **{key: value})
            with self.assertRaises(ValueError):
                read_capture("COAPTIC_DEVICE " + json.dumps(invalid), case)


if __name__ == "__main__":
    unittest.main()
