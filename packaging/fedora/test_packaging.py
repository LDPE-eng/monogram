#!/usr/bin/env python3
import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def read(path):
    return (ROOT / path).read_text(encoding="utf-8")


class PackagingContractTests(unittest.TestCase):
    def test_rpm_license_covers_bundled_mit_and_mpl_code(self):
        spec = read("packaging/fedora/monogram-desktop.spec")
        self.assertRegex(
            spec,
            r"(?m)^License:\s+GPL-3\.0-only AND MIT AND MPL-2\.0$",
        )

    def test_appstream_license_matches_rpm_license(self):
        metadata = read("packaging/fedora/io.github.monogram.desktop.metainfo.xml")
        self.assertIn(
            "<project_license>GPL-3.0-only AND MIT AND MPL-2.0</project_license>",
            metadata,
        )

    def test_mono_rs_third_party_notices_are_installed_as_license(self):
        spec = read("packaging/fedora/monogram-desktop.spec")
        self.assertRegex(
            spec,
            r"(?m)^%license vendor/mono-rs/docs/THIRD_PARTY_NOTICES\.md$",
        )

    def test_tlottie_changes_trigger_desktop_ci(self):
        workflow = read(".github/workflows/desktop.yml")
        push = workflow.split("  push:\n", 1)[1].split("  pull_request:\n", 1)[0]
        pull_request = workflow.split("  pull_request:\n", 1)[1].split(
            "  workflow_dispatch:", 1
        )[0]
        self.assertIn("- 'vendor/tlottie'", push)
        self.assertIn("- 'vendor/tlottie'", pull_request)

    def test_ci_formatting_is_scoped_to_the_desktop_package(self):
        workflow = read(".github/workflows/desktop.yml")
        format_step = re.search(
            r"(?ms)^      - name: Format\n.*?(?=^      - name:)", workflow
        )
        self.assertIsNotNone(format_step)
        self.assertIn(
            "run: cargo fmt --package monogram-desktop -- --check",
            format_step.group(0),
        )
        self.assertNotIn("cargo fmt --all", format_step.group(0))

    def test_readme_lists_headless_test_and_udev_build_dependencies(self):
        readme = read("desktop/README.md")
        self.assertIn("systemd-devel", readme)
        self.assertIn("/usr/bin/kwin_wayland", readme)
        self.assertIn("/usr/bin/dbus-run-session", readme)

    def test_python_bytecode_is_ignored(self):
        gitignore = read("desktop/.gitignore").splitlines()
        self.assertIn("__pycache__/", gitignore)
        self.assertIn("*.pyc", gitignore)


if __name__ == "__main__":
    unittest.main()
