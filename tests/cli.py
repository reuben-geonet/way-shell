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

    def run_without_server(self, args, runtime=None):
        env = dict(self.env)
        env.pop("XDG_RUNTIME_DIR")
        if runtime is not None:
            env["XDG_RUNTIME_DIR"] = runtime
        return subprocess.run([BINARY, *args], env=env, capture_output=True, timeout=3)

    def test_help_without_server(self):
        for args in [[], ["--help"], ["-h"], ["volume"], ["volume", "--help"],
                     ["volume", "set", "--help"]]:
            with self.subTest(args=args):
                result = self.run_without_server(args)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue(result.stdout)

    def test_invalid_arguments_without_server(self):
        arguments = [["unknown"], ["volume", "unknown"], ["volume", "set"],
                     ["volume", "up", "extra"], ["volume", "set", "0", "1"]]
        arguments += [["volume", "set", value] for value in
                      ["", "abc", "0.5oops", "NaN", "inf", "-inf", "-0.1", "1.1", "1e999", "1e-999"]]
        for args in arguments:
            with self.subTest(args=args):
                result = self.run_without_server(args)
                self.assertEqual(result.returncode, 2, result.stderr)

    def test_absent_server_and_bad_path(self):
        for path in [None, "", "/does/not/exist", "/" + "x" * 200, "relative"]:
            with self.subTest(path=path):
                self.assertEqual(self.run_without_server(["volume", "up"], path).returncode, 1)

    def test_concurrent_clients(self):
        clients = [subprocess.Popen([BINARY, "volume", "up"], env=self.env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                   for _ in range(8)]
        addresses = set()
        try:
            for _ in clients:
                data, address = self.server.recvfrom(1024)
                self.assertEqual(data, struct.pack("<I", 2))
                self.assertNotIn(address, addresses)
                addresses.add(address)
                self.server.sendto(struct.pack("<I", 1), address)
            for client in clients:
                stdout, stderr = client.communicate(timeout=3)
                self.assertEqual(client.returncode, 0, (stdout, stderr))
        finally:
            for client in clients:
                if client.poll() is None:
                    client.kill()
                client.wait()
                client.stdout.close()
                client.stderr.close()

    def test_response_deadline(self):
        started = time.monotonic()
        with subprocess.Popen([BINARY, "volume", "up"], env=self.env,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE) as client:
            try:
                self.server.recvfrom(1024)
                client.communicate(timeout=3)
                self.assertEqual(client.returncode, 1)
                self.assertGreater(time.monotonic() - started, 1.8)
                self.assertLess(time.monotonic() - started, 3)
            finally:
                if client.poll() is None:
                    client.kill()
                    client.wait()


if __name__ == "__main__":
    unittest.main()
