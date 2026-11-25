"""A build step that downloads a tool from the internet must retry, so one 503 from a CDN cannot turn a good push red.

On 2026-11-25 the Linux bundle job compiled the app and built the deb and rpm, then failed on `http status: 503` while
fetching linuxdeploy for the AppImage. Nothing was wrong with the code. The bundle step now loops three times; this test keeps
it that way. Standard library only; run from the repository root: python3 -m unittest discover -s scripts/tests -t .
"""
import unittest
from pathlib import Path

CI = Path(__file__).resolve().parent.parent.parent / ".github" / "workflows" / "ci.yml"


def step_block(text, name):
    """The lines of the step called `name`, up to the next step."""
    lines = text.split("\n")
    start = next(i for i, line in enumerate(lines) if line.strip() == f"- name: {name}")
    end = start + 1
    while end < len(lines) and not lines[end].startswith("      - "):
        end += 1
    return "\n".join(lines[start:end])


class BundleStepRetries(unittest.TestCase):
    def setUp(self):
        self.step = step_block(CI.read_text(encoding="utf-8"), "Build the bundles")

    def test_it_tries_more_than_once(self):
        self.assertIn("for attempt in 1 2 3", self.step)

    def test_it_stops_at_the_first_success(self):
        self.assertIn("&& exit 0", self.step)

    def test_it_fails_when_every_attempt_failed(self):
        self.assertTrue(self.step.rstrip().endswith("exit 1"), "after the loop the step must fail, or a broken build would pass")

    def test_it_still_builds_all_three_bundle_types(self):
        self.assertIn("--bundles deb,rpm,appimage", self.step)


if __name__ == "__main__":
    unittest.main()
