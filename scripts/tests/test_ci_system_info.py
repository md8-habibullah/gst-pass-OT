"""Every CI job starts by printing what machine it runs on (OS, CPU, memory, disk, tool versions), and nothing secret.

The step is the same text in every job of .github/workflows/*.yml (a workflow cannot share a step between jobs without checking
the repository out first, and the step must run before anything else). This keeps the copies identical, first in their job, unable
to fail a build, and free of anything that could print a secret. Standard library only; run from the repository root:
python3 -m unittest discover -s scripts/tests -t .
"""
import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = sorted((ROOT / ".github" / "workflows").glob("*.yml"))
STEP_NAME = "System details (public information only)"
# nothing that reads a secret, dumps the environment, or traces commands may be in the step
FORBIDDEN = ("secrets", "TOKEN", "printenv", "${{", "set -x", "export ", "env |", " env\n", "declare -p", "compgen -e")


def jobs_with_steps(path):
    """(job id, lines of its steps: list) for every job, found without a YAML library: jobs sit at two spaces, `steps:` at four."""
    lines = path.read_text(encoding="utf-8").split("\n")
    out, job, in_jobs, i = [], None, False, 0
    while i < len(lines):
        line = lines[i]
        if line == "jobs:":
            in_jobs = True
        elif in_jobs and re.fullmatch(r"  [A-Za-z0-9_-]+:", line):
            job = line.strip()[:-1]
        elif in_jobs and job and line == "    steps:":
            j = i + 1
            while j < len(lines) and (lines[j].startswith("      ") or lines[j].strip() == "" or lines[j].startswith("        ")):
                j += 1
            out.append((job, lines[i + 1 : j]))
            i = j - 1
        i += 1
    return out


def first_step(step_lines):
    """The first list item of a steps block, comments above it included."""
    out, started = [], False
    for line in step_lines:
        if line.startswith("      - "):
            if started:
                break
            started = True
        out.append(line)
    return "\n".join(out).rstrip()


class SystemDetailsStep(unittest.TestCase):
    def test_every_job_starts_with_the_step(self):
        seen = 0
        for path in WORKFLOWS:
            for job, steps in jobs_with_steps(path):
                with self.subTest(file=path.name, job=job):
                    first = first_step(steps)
                    self.assertIn(f"- name: {STEP_NAME}", first, "the first step of every job is the system details")
                    self.assertIn("continue-on-error: true", first, "printing details must never fail a build")
                    seen += 1
        self.assertGreaterEqual(seen, 12, "ci.yml has 9 jobs and pages.yml 3")

    def test_all_copies_are_identical(self):
        copies = {}
        for path in WORKFLOWS:
            for job, steps in jobs_with_steps(path):
                copies.setdefault(first_step(steps), []).append(f"{path.name}:{job}")
        self.assertEqual(len(copies), 1, "the system-details step differs between jobs: " + str(list(copies.values())))

    def test_it_prints_no_secret_and_dumps_no_environment(self):
        step = first_step(jobs_with_steps(WORKFLOWS[0])[0][1])
        for bad in FORBIDDEN:
            self.assertNotIn(bad, step, f"{bad!r} must not appear in the system-details step")

    def test_the_script_runs_and_shows_the_facts_but_no_environment_variable(self):
        step = first_step(jobs_with_steps(WORKFLOWS[0])[0][1])
        body = step.split("        run: |\n", 1)[1]
        script = "\n".join(l[10:] if l.startswith(" " * 10) else l for l in body.split("\n")) + "\n"
        with tempfile.NamedTemporaryFile("r") as summary:
            env = {"PATH": os.environ["PATH"], "GITHUB_JOB": "demo", "RUNNER_OS": "Linux", "RUNNER_ARCH": "X64",
                   "GITHUB_SHA": "0123abc", "HEXIS_TEST_SECRET": "do-not-print-me", "GITHUB_TOKEN": "do-not-print-me-either",
                   "GITHUB_STEP_SUMMARY": summary.name}
            run = subprocess.run(["bash", "-e", "-c", script], capture_output=True, text=True, env=env, timeout=60)
            self.assertEqual(run.returncode, 0, run.stderr)
            for label in ("### System: demo on Linux X64", "time ", "system ", "cpu ", "memory ", "disk ", "load "):
                self.assertIn(label, run.stdout)
            self.assertIn("0123abc", run.stdout)
            self.assertNotIn("do-not-print-me", run.stdout + run.stderr)
            self.assertEqual(summary.read(), run.stdout, "the run page shows what the log shows")


class ContainerBanner(unittest.TestCase):
    def test_every_installer_container_says_what_it_is_before_anything_runs(self):
        src = (ROOT / "scripts" / "tests" / "container_case.sh").read_text(encoding="utf-8")
        banner = src.index("container:")
        self.assertLess(banner, src.index("apt-get update"), "the banner comes before the first install")
        for needle in ("/etc/os-release", "uname -r", "MemTotal", "memory.max", "df -h /"):
            self.assertIn(needle, src[banner : banner + 1500])


if __name__ == "__main__":
    unittest.main()
