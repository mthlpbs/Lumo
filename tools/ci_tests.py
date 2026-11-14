"""Build Rust tests once, then run every non-ignored test in a fresh process.

Windows native libraries can terminate a process without a Rust panic. Keep
those failures attributable to one test; never retry, skip, or hide them.
Only Python's standard library and the installed Cargo toolchain are needed.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
TEST_RESULT = re.compile(
    r"^test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored;", re.MULTILINE,
)


def invoke(command, timeout, cwd=ROOT):
    try:
        result = subprocess.run(
            command, cwd=cwd, capture_output=True, text=True,
            encoding="utf-8", errors="replace", timeout=timeout,
        )
        return result.returncode, result.stdout + result.stderr
    except subprocess.TimeoutExpired as error:
        def decoded(value):
            return value.decode("utf-8", errors="replace") if isinstance(value, bytes) else (value or "")
        output = decoded(error.stdout) + decoded(error.stderr)
        return None, f"{output}\nTimed out after {timeout} seconds.\n"


def test_names(output):
    names = re.findall(r"^(\S+): test\s*$", output, re.MULTILINE)
    if len(names) != len(set(names)):
        raise RuntimeError("test listing contains duplicate names")
    return names


def run_binary(command, out, timeout=60):
    """Return results for all tests, including the ones libtest marks ignored."""
    out.mkdir(parents=True, exist_ok=True)
    listings = []
    for flags in (["--list"], ["--list", "--ignored"]):
        code, output = invoke([*command, *flags], timeout)
        listing = "ignored-list.log" if "--ignored" in flags else "list.log"
        (out / listing).write_text(output, encoding="utf-8")
        if code != 0:
            raise RuntimeError(f"could not list tests (exit {code}):\n{output}")
        listings.append(set(test_names(output)))
    names, ignored = listings
    if not ignored <= names:
        raise RuntimeError("ignored test listing does not match the full listing")
    results = []
    for name in sorted(names):
        if name in ignored:
            results.append({"name": name, "status": "ignored"})
            continue
        started = time.monotonic()
        code, output = invoke(
            [*command, name, "--exact", "--nocapture", "--test-threads=1"], timeout,
        )
        summaries = TEST_RESULT.findall(output)
        passed = code == 0 and summaries and summaries[-1] == ("ok", "1", "0", "0")
        status = "passed" if passed else "failed"
        hex_code = f"0x{code & 0xffffffff:08X}" if code is not None else None
        exit_detail = f"{code} ({hex_code})" if code is not None else "timeout"
        log = out / (hashlib.sha256(name.encode()).hexdigest()[:16] + ".log")
        log.write_text(f"Test: {name}\nExit: {exit_detail}\n{output}", encoding="utf-8")
        results.append({
            "name": name, "status": status, "exit_code": code, "exit_code_hex": hex_code,
            "seconds": round(time.monotonic() - started, 3), "log": str(log),
        })
        print(f"{status.upper()}: {name}", flush=True)
        if status == "failed":
            print(f"exit {exit_detail}\n{output}", flush=True)
    return results


def build_tests(cargo_args, out):
    command = ["cargo", "test", *cargo_args, "--no-run", "--message-format=json-render-diagnostics"]
    print("Building test executables: " + " ".join(command), flush=True)
    # Stream compiler diagnostics so a long clean build remains visible in CI.
    executables = {}
    with (out / "build.log").open("w", encoding="utf-8") as log:
        with subprocess.Popen(
            command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", errors="replace",
        ) as build:
            for line in build.stdout:
                log.write(line)
                try:
                    event = json.loads(line)
                except ValueError:
                    print(line, end="", flush=True)
                    continue
                if event.get("reason") == "compiler-message":
                    print(event["message"].get("rendered", ""), end="", flush=True)
                if event.get("reason") == "compiler-artifact":
                    executable = event.get("executable")
                    if executable and event.get("profile", {}).get("test"):
                        executables[executable] = event["target"]
            if build.wait() != 0:
                raise RuntimeError("Cargo test compilation failed; see build.log")
    if not executables:
        raise RuntimeError("Cargo produced no test executables")
    return executables


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", default="x86_64-pc-windows-msvc")
    parser.add_argument("--package", action="append", default=[])
    parser.add_argument("--out", type=Path, default=ROOT / "test-out")
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args(argv)
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    cargo_args = ["--locked", "--target", args.target]
    for package in args.package:
        cargo_args.extend(["--package", package])
    if not args.package:
        cargo_args.append("--workspace")
    if args.offline:
        cargo_args.append("--offline")
    report = {"source": os.environ.get("GITHUB_SHA", "local"), "target": args.target, "suites": []}
    try:
        executables = build_tests(cargo_args, args.out)
        for executable, target in sorted(executables.items()):
            suite = {"executable": executable, "tests": []}
            report["suites"].append(suite)
            folder = args.out / (Path(executable).stem + "-cases")
            suite["tests"] = run_binary([executable], folder, args.timeout)
        tests = [test for suite in report["suites"] for test in suite["tests"]]
        report["passed"] = sum(test["status"] == "passed" for test in tests)
        report["failed"] = sum(test["status"] == "failed" for test in tests)
        report["ignored"] = sum(test["status"] == "ignored" for test in tests)
        if not report["passed"] and not report["failed"]:
            raise RuntimeError("no runnable tests were discovered")
        # Cargo's --no-run has no executable artifacts for documentation tests.
        if any("lib" in target["kind"] for target in executables.values()):
            code, output = invoke(["cargo", "test", *cargo_args, "--doc"], 300)
            (args.out / "doctests.log").write_text(output, encoding="utf-8")
            print(output, flush=True)
            report["doctests_exit_code"] = code
        print(f"Tests: {report['passed']} passed, {report['failed']} failed, {report['ignored']} ignored.", flush=True)
        return int(bool(report["failed"]) or report.get("doctests_exit_code", 0) != 0)
    except (OSError, RuntimeError) as error:
        report["error"] = str(error)
        print(f"ERROR: {error}", file=sys.stderr, flush=True)
        return 1
    finally:
        (args.out / "results.json").write_text(json.dumps(report, indent=2), encoding="utf-8")


if __name__ == "__main__":
    sys.exit(main())
