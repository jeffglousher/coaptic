import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "qualification"))
from esphome_probe import configuration


class ConfigurationRefusalTests(unittest.TestCase):
    def test_unprepared_variants_invalid_identity_and_other_toolchains_refuse(self):
        for field, value, message in [
            ("variant", "esp32h2", "only available on ESP32C3, ESP32C6"),
            ("framework", {"type": "arduino"}, "only available with framework(s) esp-idf"),
            ("toolchain", "platformio", "requires the native esp-idf toolchain"),
            ("run_id", "stale", "32 lowercase hexadecimal characters"),
        ]:
            with self.subTest(field=field), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                config = configuration("esp32c3", False, "a" * 32, root / "build")
                owner = config["coaptic_probe"] if field == "run_id" else config["esp32"]
                owner[field] = value
                path = root / "probe.yaml"
                path.write_text(json.dumps(config), encoding="utf-8")
                result = subprocess.run([sys.executable, "-m", "esphome", "config", str(path)],
                                        capture_output=True, text=True, timeout=60)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
