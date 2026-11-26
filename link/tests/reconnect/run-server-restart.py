#!/usr/bin/env python3
"""Exercise both SDKs across a restart of an isolated local KalamDB server."""

import os
import pathlib
import signal
import socket
import subprocess
import tempfile
import time
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[3]
SERVER = pathlib.Path(os.environ.get("KALAM_SERVER_BINARY", ROOT / "target/debug/kalamdb-server"))
BRIDGE = pathlib.Path(os.environ.get("KALAM_LINK_TEST_LIBRARY", ROOT / "target/debug/libkalam_link_dart.dylib"))


def wait_until(predicate, timeout=60):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.1)
    raise TimeoutError("restart fixture did not become ready")


def main():
    if not SERVER.is_file() or not BRIDGE.is_file():
        raise RuntimeError("Build kalamdb-server, kalam-link-dart, and the TypeScript SDK first")

    with tempfile.TemporaryDirectory(prefix="kalam-restart-") as directory:
        base = pathlib.Path(directory)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        url = f"http://127.0.0.1:{port}"
        config = (ROOT / "backend/server.example.toml").read_text()
        config = config.replace('host = "0.0.0.0"', 'host = "127.0.0.1"', 1)
        config = config.replace("port = 2900", f"port = {port}", 1)
        config = config.replace('data_path = "./data"', f'data_path = "{base}/data"', 1)
        config = config.replace('# root_password = "change-me"', 'root_password = "restart-fixture-password"', 1)
        (base / "server.toml").write_text(config)
        env = os.environ | {
            "KALAM_INTEGRATION_TEST": "1",
            "KALAM_BUILD_DART_BRIDGE": "0",
            "KALAMDB_URL": url,
            "KALAMDB_USER": "root",
            "KALAMDB_PASSWORD": "restart-fixture-password",
            "KALAM_RESTART_COORD_DIR": str(base),
            "KALAM_LINK_TEST_LIBRARY": str(BRIDGE),
        }
        processes = []
        log = (base / "server.log").open("w")

        def start_server():
            server = subprocess.Popen([str(SERVER), str(base / "server.toml")], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
            processes.append(server)
            def healthy():
                if server.poll() is not None:
                    raise RuntimeError("server exited: " + (base / "server.log").read_text())
                try:
                    with urllib.request.urlopen(url + "/health", timeout=0.5):
                        return True
                except Exception:
                    return False
            wait_until(healthy)
            return server

        try:
            server = start_server()
            dart = subprocess.Popen(
                ["flutter", "test", "--no-pub", "test/e2e/reconnect/server_restart_test.dart", "--reporter", "expanded"],
                cwd=ROOT / "link/sdks/dart/link", env=env,
            )
            ts = subprocess.Popen(["node", str(ROOT / "link/tests/reconnect/server-restart.mjs")], env=env)
            processes.extend([dart, ts])
            def ready():
                if dart.poll() is not None or ts.poll() is not None:
                    raise RuntimeError("SDK fixture exited before restart")
                return (base / "dart-ready").exists() and (base / "ts-ready").exists()
            wait_until(ready)
            server.send_signal(signal.SIGINT)
            if server.wait(timeout=20) != 0:
                raise RuntimeError("server shutdown failed")
            start_server()
            (base / "restarted").touch()
            codes = (dart.wait(timeout=90), ts.wait(timeout=90))
            if any(codes):
                raise RuntimeError(f"restart tests failed: Dart={codes[0]}, TypeScript={codes[1]}")
        finally:
            for process in processes:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
            log.close()


if __name__ == "__main__":
    main()
