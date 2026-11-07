"""Exercise the actual CI tag-classification and version-validation scripts.

Run with python tools/test_release_workflow.py (Python 3.11+, no dependencies).
"""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import tomllib
import unittest

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
VERSION = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"]


def step_script(name):
    step = WORKFLOW.split(f"      - name: {name}\n", 1)[1]
    lines = step.split("        run: |\n", 1)[1].splitlines()
    code = []
    for line in lines:
        if line.strip() and not line.startswith("          "):
            break
        code.append(line)
    return textwrap.dedent("\n".join(code))


def run_step(name, tag, event="release"):
    with tempfile.TemporaryDirectory(prefix="lumo-ci-tag-") as directory:
        output = Path(directory) / "output.txt"
        result = subprocess.run(
            [sys.executable, "-c", step_script(name)], cwd=ROOT,
            env={**os.environ, "EVENT_NAME": event, "RELEASE_TAG": tag, "GITHUB_OUTPUT": str(output)},
            capture_output=True, text=True,
        )
        return result, output.read_text() if output.exists() else ""


class ReleaseWorkflowTests(unittest.TestCase):
    def test_numeric_and_historical_tags_build(self):
        for tag in [VERSION, f"v{VERSION}", "2026.1107.1", "v1.0.0"]:
            with self.subTest(tag=tag):
                result, output = run_step("Identify app release", tag)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(output.strip(), "build=true")

    def test_addons_and_invalid_tags_stay_skipped(self):
        for tag in ["dictionary-data", "ja-dict-2026-11", "v-data", "", "2026.1107.2-preview", "2026.1107.2/path"]:
            with self.subTest(tag=tag):
                result, output = run_step("Identify app release", tag)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(output.strip(), "build=false")

    def test_manual_dispatch_builds_without_a_tag(self):
        result, output = run_step("Identify app release", "", "workflow_dispatch")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output.strip(), "build=true")

    def test_both_current_tag_formats_validate(self):
        for tag in [VERSION, f"v{VERSION}", ""]:
            with self.subTest(tag=tag):
                result, output = run_step("Validate release version", tag)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(output.strip(), f"version={VERSION}")

    def test_mismatched_versions_fail_with_the_current_version(self):
        for tag in ["2026.1107.1", "v1.0.0"]:
            if tag.removeprefix("v") == VERSION:
                continue
            with self.subTest(tag=tag):
                result, output = run_step("Validate release version", tag)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(f"must match Cargo.toml version {VERSION}", result.stderr)
                self.assertEqual(output, "")


if __name__ == "__main__":
    unittest.main()
