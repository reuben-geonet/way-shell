"""Black-box protocol characterization; no desktop session is used."""

import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import time
import unittest


BINARY = str(Path(os.environ.get("WAY_SH", "way-sh/way-sh")).resolve())
FIXTURES = json.loads(Path("tests/fixtures/cli.json").read_text())


class Cli(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="way-sh-test-")
        self.addCleanup(self.directory.cleanup)
        self.env = {**os.environ, "XDG_RUNTIME_DIR": self.directory.name}
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM)
        self.addCleanup(self.server.close)
        self.server.bind(self.directory.name + "/way-shell.sock")
        self.server.settimeout(3)

    def exchange(self, args, response=1):
        # Baseline client names contain only whole seconds. Remove this delay
        # when the dedicated CLI fix introduces unique client addresses.
        time.sleep(1.01)
        with subprocess.Popen([BINARY, *args], env=self.env,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE) as client:
            try:
                message, address = self.server.recvfrom(1024)
                encoded = response if isinstance(response, bytes) else struct.pack("<I", response)
                self.server.sendto(encoded, address)
                stdout, stderr = client.communicate(timeout=3)
            finally:
                if client.poll() is None:
                    client.kill()
                    client.wait()
        return message, client.returncode, stdout, stderr

    def test_valid_commands(self):
        for fixture in FIXTURES:
            with self.subTest(args=fixture["args"]):
                expected = struct.pack("<I", fixture["opcode"])
                if "volume" in fixture:
                    expected += struct.pack("<f", fixture["volume"])
                message, status, stdout, stderr = self.exchange(fixture["args"])
                self.assertEqual(message, expected)
                self.assertEqual(status, 0, (stdout, stderr))

    def test_server_failure(self):
        _, status, _, _ = self.exchange(["volume", "up"], response=0)
        self.assertEqual(status, 1)

    def test_malformed_replies(self):
        for reply in [b"", b"\x01", b"\x01\x00\x00\x00\x00", struct.pack("<I", 2)]:
            with self.subTest(reply=reply):
                _, status, _, _ = self.exchange(["volume", "up"], response=reply)
                self.assertEqual(status, 1)


if __name__ == "__main__":
    unittest.main()
