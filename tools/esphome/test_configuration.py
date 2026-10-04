import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "qualification"))
from esphome_probe import configuration


class ConfigurationRefusalTests(unittest.TestCase):
    def test_bundled_network_refuses_wrong_chip_and_corrupt_archive(self):
        for chip, corrupt, message in [("esp32c3", False, "prepared only for ESP32-S3"),
                                       ("esp32s3", True, "archive checksum mismatch")]:
            with self.subTest(chip=chip), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                config = configuration(chip, False, "a" * 32, root / "build")
                config["external_components"][0]["components"] = ["coaptic_network"]
                del config["coaptic_probe"]
                config["coaptic_network"] = {"run_id": "a" * 32, "qualification_only": True,
                                              "rust_runtime": "bundled"}
                config["wifi"] = {"ssid": "qualification-test"}
                if corrupt:
                    source = Path(__file__).resolve().parent / "components/coaptic_network"
                    shutil.copytree(source, root / "components/coaptic_network")
                    config["external_components"][0]["source"]["path"] = str(root / "components")
                    with (root / "components/coaptic_network/lib/libcoaptic_esphome_probe.a").open("ab") as archive:
                        archive.write(b"corruption")
                path = root / "network.yaml"
                path.write_text(json.dumps(config), encoding="utf-8")
                result = subprocess.run([sys.executable, "-m", "esphome", "config", str(path)],
                                        capture_output=True, text=True, timeout=60)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(message, result.stdout + result.stderr)

    def test_bundled_and_external_network_runtimes_generate_without_replacing_component_dirs(self):
        components = Path(__file__).resolve().parent / "components"
        for runtime in ["bundled", "external"]:
            with self.subTest(runtime=runtime), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                config = configuration("esp32s3", False, "a" * 32, root / "build")
                config["external_components"][0]["components"] = ["coaptic_network"]
                del config["coaptic_probe"]
                config["coaptic_network"] = {"run_id": "a" * 32, "qualification_only": True,
                                              "rust_runtime": runtime}
                config["wifi"] = {"ssid": "qualification-test", "reboot_timeout": "0s"}
                path = root / "network.yaml"
                path.write_text(json.dumps(config), encoding="utf-8")
                result = subprocess.run([sys.executable, "-m", "esphome", "compile", str(path),
                                         "--only-generate"], capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                cmake = (root / "build/CMakeLists.txt").read_text()
                self.assertNotIn("coaptic_rust_probe", cmake)
                self.assertNotIn("COAPTIC_RUST_MANIFEST", cmake)
                manifest = (root / "build/src/idf_component.yml").read_text()
                if runtime == "bundled":
                    self.assertIn("coaptic_rust_network", manifest)
                    self.assertIn((components / "coaptic_network/coaptic_rust_network").as_posix(), manifest.replace("\\", "/"))
                else:
                    self.assertNotIn("coaptic_rust_network", manifest)

    def test_unprepared_variants_invalid_identity_and_other_toolchains_refuse(self):
        for field, value, message in [
            ("variant", "esp32h2", "only available on ESP32C3, ESP32C6, ESP32S3"),
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
