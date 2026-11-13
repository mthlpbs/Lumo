"""Verify the CI runner detects native exits, timeouts, and empty test runs."""

from pathlib import Path
import sys
import tempfile
import unittest

from ci_tests import run_binary, test_names


class TestRunnerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="lumo-ci-runner-")
        self.root = Path(self.temporary.name)
        self.harness = self.root / "harness.py"

    def tearDown(self):
        self.temporary.cleanup()

    def run_harness(self, body, timeout=3):
        self.harness.write_text(body, encoding="utf-8")
        return run_binary([sys.executable, str(self.harness)], self.root / "logs", timeout)

    def test_processes_are_fresh_and_ignored_tests_never_execute(self):
        results = self.run_harness('''
import sys
if "--list" in sys.argv:
    print("ignored: test")
    if "--ignored" not in sys.argv:
        print("first: test\\nsecond: test")
else:
    assert sys.argv[1] != "ignored"
    assert "--exact" in sys.argv
    import builtins
    assert not hasattr(builtins, "seen_test")
    builtins.seen_test = True
    print("test result: ok. 1 passed; 0 failed; 0 ignored;")
''')
        self.assertEqual([r["status"] for r in results], ["passed", "ignored", "passed"])

    def test_native_exit_is_a_failure_and_remaining_tests_still_run(self):
        results = self.run_harness('''
import os, sys
if "--list" in sys.argv:
    if "--ignored" not in sys.argv:
        print("crash: test\\nlater: test")
elif sys.argv[1] == "crash":
    os._exit(23)
else:
    print("test result: ok. 1 passed; 0 failed; 0 ignored;")
''')
        self.assertEqual(results[0]["status"], "failed")
        self.assertEqual(results[0]["exit_code"], 23)
        self.assertEqual(results[1]["status"], "passed")
        self.assertIn("Exit: 23", Path(results[0]["log"]).read_text())

    def test_zero_tests_cannot_report_success(self):
        results = self.run_harness('''
import sys
if "--list" in sys.argv:
    if "--ignored" not in sys.argv:
        print("renamed: test")
else:
    print("test result: ok. 0 passed; 0 failed; 0 ignored;")
''')
        self.assertEqual(results[0]["status"], "failed")

    def test_timeout_is_a_failure(self):
        results = self.run_harness('''
import sys, time
if "--list" in sys.argv:
    if "--ignored" not in sys.argv:
        print("hang: test")
else:
    print("before timeout", flush=True)
    time.sleep(30)
''', timeout=1)
        self.assertEqual(results[0]["status"], "failed")
        self.assertIsNone(results[0]["exit_code"])
        self.assertIn("before timeout", Path(results[0]["log"]).read_text())

    def test_listing_failure_is_not_an_empty_successful_suite(self):
        with self.assertRaisesRegex(RuntimeError, "could not list tests"):
            self.run_harness("import sys; sys.exit(19)")

    def test_duplicate_names_are_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "duplicate"):
            test_names("same: test\nsame: test\n")


if __name__ == "__main__":
    unittest.main()
