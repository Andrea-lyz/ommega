#!/usr/bin/env python3
"""Loopback-only synthetic A/server/B benchmark; never connects to a device.
Uses the actual server binary, an HTTP A stub and a fixed 20ms software B stub.
Two poll strategies on the SAME candidate server isolate cadence savings.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import http.client
import json
import os
from pathlib import Path
import random
import socket
import ssl
import statistics
import subprocess
import tempfile
import threading
import time

TOKEN = "loopback-benchmark-only"

def exchange(connection, method, path, body=None):
    data = None if body is None else json.dumps(body)
    connection.request(method, path, data, {"X-Relay-Token": TOKEN, "Content-Type": "application/json"})
    response = connection.getresponse()
    raw = response.read()
    return response.status, json.loads(raw) if raw else None

def run(binary, mode, samples, tls=False):
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    with tempfile.TemporaryDirectory(prefix="ommega-link-bench-") as directory:
        env = dict(os.environ, RELAY_BIND="127.0.0.1", RELAY_HTTP_PORT=str(port),
                   RELAY_USE_TLS="false", RELAY_TOKEN=TOKEN, RELAY_MYSQL_URL="",
                   RELAY_AUTH_REQUIRED="true", RUST_LOG="warn")
        tls_context = None
        if tls:
            with socket.socket() as sock:
                sock.bind(("127.0.0.1", 0))
                secure_port = sock.getsockname()[1]
            cert = Path(directory) / "cert.pem"
            key = Path(directory) / "key.pem"
            subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                "-sha256", "-days", "1", "-subj", "/CN=localhost",
                "-addext", "subjectAltName=IP:127.0.0.1", "-keyout", str(key),
                "-out", str(cert)], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            env.update(RELAY_USE_TLS="true", RELAY_HTTPS_PORT=str(secure_port),
                       RELAY_TLS_CERTFILE=str(cert), RELAY_TLS_KEYFILE=str(key))
            tls_context = ssl.create_default_context(cafile=str(cert))
        def connect(timeout):
            if tls:
                return http.client.HTTPSConnection("127.0.0.1", secure_port, timeout=timeout, context=tls_context)
            return http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
        log = open(Path(directory) / "server.log", "wb")
        process = subprocess.Popen([str(binary)], cwd=directory, env=env, stdout=log, stderr=log)
        stop, ready = threading.Event(), threading.Event()
        errors, pickups = [], {}
        counts = {"poll": 0}
        executor = ThreadPoolExecutor(max_workers=3)
        def execute(task):
            pickups[task["payload"]["request_id"]] = time.perf_counter()
            time.sleep(.020)  # explicitly synthetic work, NOT hardware or crypto
            connection = connect(5)
            try:
                status, _ = exchange(connection, "POST", "/api/b/result/", {
                    "task_id": task["task_id"], "device_id": "synthetic-b",
                    "result": {"signature": "AQID"}})
                assert status == 200, status
            except Exception as error:
                errors.append(repr(error))
            finally:
                connection.close()
        def poll():
            connection = connect(25)
            try:
                status, _ = exchange(connection, "GET", "/api/b/poll/?device_id=synthetic-b&machine_id=mock&timeout=0&caps=")
                assert status == 204, status
                ready.set()
                while not stop.is_set():
                    started = time.perf_counter()
                    counts["poll"] += 1
                    timeout = 0 if mode == "short80" else 20
                    status, task = exchange(connection, "GET", f"/api/b/poll/?device_id=synthetic-b&machine_id=mock&timeout={timeout}&caps=")
                    if status == 200:
                        executor.submit(execute, task)
                    else:
                        assert status == 204, (status, task)
                        stop.wait(max(0., .080 - (time.perf_counter() - started)))
            except Exception as error:
                if not stop.is_set():
                    errors.append(repr(error))
                    ready.set()
            finally:
                connection.close()
        thread = None
        try:
            deadline = time.monotonic() + 10
            while True:
                if process.poll() is not None:
                    raise RuntimeError("mock server exited before readiness")
                try:
                    connection = connect(.2)
                    connection.request("GET", "/api/ping/")
                    response = connection.getresponse()
                    assert response.read() == b"pong"
                    connection.close()
                    break
                except (OSError, http.client.HTTPException):
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(.02)
            thread = threading.Thread(target=poll, daemon=True)
            thread.start()
            assert ready.wait(5), "B stub never became ready"
            assert not errors, errors
            before = counts["poll"]
            time.sleep(1)
            idle_polls = counts["poll"] - before
            rng = random.Random(214)
            rtts, pickup_ms = [], []
            a = connect(10)
            for index in range(samples + 3):
                time.sleep(rng.uniform(0., .080))
                request = f"synthetic-{mode}-{index}"
                started = time.perf_counter()
                status, result = exchange(a, "POST", "/api/sign/", {
                    "device_id": "synthetic-b", "request_id": request, "alias": "mock", "data": "AQID"})
                elapsed = (time.perf_counter() - started) * 1000
                assert status == 200 and result["signature"] == "AQID", (status, result)
                if index >= 3:
                    rtts.append(elapsed)
                    pickup_ms.append((pickups[request] - started) * 1000)
            a.close()
            assert not errors, errors
            def summary(values):
                ordered = sorted(values)
                return {"p50_ms": round(statistics.median(ordered), 3),
                        "p95_ms": round(ordered[min(len(ordered)-1, int(.95*len(ordered)))], 3),
                        "min_ms": round(ordered[0], 3), "max_ms": round(ordered[-1], 3)}
            return {"mode": mode, "transport": "HTTPS verified fixture CA" if tls else "HTTP", "samples": samples, "software_work_ms": 20,
                    "idle_polls_in_one_second": idle_polls, "roundtrip": summary(rtts),
                    "request_to_stub_pickup": summary(pickup_ms)}
        finally:
            stop.set()
            process.terminate()  # only our local synthetic server; no secure calls
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
            if thread is not None:
                thread.join(timeout=3)
            executor.shutdown(wait=True)
            log.close()

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--samples", type=int, default=60)
    parser.add_argument("--tls", action="store_true", help="Use Rustls listener and an ephemeral trusted loopback certificate")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    assert 1 <= args.samples <= 1000
    result = {"scope": "loopback HTTP(S) stubs; SAME candidate server; no WAN/TEE/A validation",
              "binary": str(args.binary.resolve()),
              "results": [run(args.binary.resolve(), mode, args.samples, args.tls) for mode in ("short80", "long20")]}
    text = json.dumps(result, indent=2)
    print(text)
    if args.output:
        args.output.write_text(text + "\n", encoding="utf-8")
