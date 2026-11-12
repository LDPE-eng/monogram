#!/usr/bin/env python3
"""Offline converter and standalone Rust runtime contract checks."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET

import convert_locales as converter

COUNTS = (0, 1, 2, 3, 4, 5, 10, 11, 12, 14, 21, 22, 25, 101, 111, 1_000_000, 2**64 - 1)
ARGUMENTS = {1: 'A1{ $arg2 }<>&"\\', 2: "A2", 3: "A3", 4: "A4", 5: "A5"}


def category(locale, n):
    if locale in ("ru", "uk"):
        if n % 10 == 1 and n % 100 != 11:
            return "one"
        if n % 10 in (2, 3, 4) and n % 100 not in (12, 13, 14):
            return "few"
        return "many"
    if locale == "sk":
        return "one" if n == 1 else "few" if n in (2, 3, 4) else "other"
    if locale in ("hy", "pt-BR") and n in (0, 1):
        return "one"
    if locale in ("es", "pt-BR") and n > 0 and n % 1_000_000 == 0:
        return "many"
    if locale in ("en", "es", "tr") and n == 1:
        return "one"
    return "other"


def rust_string(text):
    hashes = "#"
    while '"' + hashes in text:
        hashes += "#"
    return f'r{hashes}"{text}"{hashes}'


class LocalizationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.output, cls.manifest = converter.generate()

    def test_complete_catalog_and_exact_checked_in_output(self):
        self.assertEqual(self.manifest["counts"], {
            "xml_files": 80, "base_strings": 1317, "base_plurals": 32,
            "locales": 10, "keys_per_locale": 1349,
            "source_strings_all_locales": 13161, "source_plurals_all_locales": 320,
        })
        for name, content in self.output.items():
            self.assertEqual((converter.ROOT / "desktop/locales" / name).read_bytes(), content, name)
        for name, digest in self.manifest["resources"].items():
            self.assertEqual(hashlib.sha256(self.output[name]).hexdigest(), digest)
        for locale in converter.LOCALES:
            parsed = converter.parse_ftl(self.output[f"{locale}/monogram.ftl"].decode())
            self.assertEqual(set(parsed), set(self.manifest["entries"]))
            self.assertEqual(sum("value" in variants for variants in parsed.values()), 1317)
            self.assertEqual(sum("value" not in variants for variants in parsed.values()), 32)
            for key, variants in parsed.items():
                info = self.manifest["entries"][key]["locales"][locale]
                if "value" not in variants:
                    self.assertEqual(set(variants), set(info["quantities"]))
                for quantity, tokens in variants.items():
                    actual = sorted(value for kind, value in tokens if kind == "variable")
                    expected = sorted(int(arg["variable"][3:]) for arg in info["signatures"][quantity])
                    self.assertEqual(actual, expected, (locale, key, quantity))

    def test_repeatability(self):
        second, manifest = converter.generate()
        self.assertEqual(self.output, second)
        self.assertEqual(self.manifest, manifest)

    def test_desktop_extra_is_complete_and_localized(self):
        expected = {
            "desktop-api-setup", "desktop-api-id", "desktop-api-hash", "desktop-api-save",
            "desktop-api-help", "desktop-connect", "desktop-chats", "desktop-search",
            "desktop-settings", "desktop-back", "desktop-send", "desktop-message",
            "desktop-retry", "desktop-loading", "desktop-empty-chats", "desktop-empty-dialog",
            "desktop-logout", "desktop-logout-confirm", "desktop-cancel", "desktop-profile",
            "desktop-appearance", "desktop-notifications", "desktop-storage", "desktop-about",
            "desktop-error-generic", "desktop-contact-first-name", "desktop-contact-last-name",
            "desktop-contact-vcard", "desktop-coordinates", "desktop-topics",
            "desktop-passkeys-help-title", "desktop-passkeys-help-text",
            "desktop-passkeys-provider-error", "desktop-diagnostics",
            "desktop-diagnostics-sub", "desktop-runtime", "desktop-runtime-sub",
            "desktop-update-check-sub", "desktop-deep-link-open-title",
            "desktop-deep-link-open-body", "desktop-deep-link-open",
            "desktop-deep-link-unsupported",
            "feature-settings-settings_show_read_status",
            "feature-settings-settings_show_read_status_sub",
            "feature-settings-settings_wallpaper_blur",
            "feature-settings-settings_wallpaper_intensity",
            "feature-settings-settings_wallpaper_rotation",
            "desktop-maintenance", "desktop-maintenance-logs",
            "desktop-maintenance-exported", "desktop-maintenance-log-count",
            "desktop-maintenance-view-logs", "desktop-maintenance-clear-logs",
            "desktop-maintenance-export-logs", "desktop-maintenance-debug-stats",
            "desktop-maintenance-version", "desktop-maintenance-not-checked",
            "desktop-maintenance-update", "desktop-maintenance-update-available",
            "desktop-maintenance-up-to-date", "desktop-maintenance-check-updates",
            "desktop-maintenance-open-installer", "desktop-maintenance-import",
            "desktop-maintenance-import-done", "desktop-maintenance-import-tdlib",
            "desktop-maintenance-import-cancel", "desktop-maintenance-crash-report",
            "desktop-maintenance-copy", "desktop-maintenance-restart",
        }
        en = converter.parse_ftl((converter.ROOT / "desktop/locales/en/desktop-extra.ftl").read_text(encoding="utf-8"))
        for locale in converter.LOCALES:
            path = converter.ROOT / "desktop/locales" / locale / "desktop-extra.ftl"
            extra = converter.parse_ftl(path.read_text(encoding="utf-8"))
            self.assertEqual(set(extra), expected, locale)
            self.assertTrue(set(extra).isdisjoint(self.manifest["entries"]))
            self.assertNotIn(f"{locale}/desktop-extra.ftl", self.output)
            for key, variants in extra.items():
                self.assertEqual(set(variants), {"value"})
                self.assertTrue(variants["value"])
                self.assertTrue(all(
                    kind == "variable" or (kind == "text" and value.strip())
                    for kind, value in variants["value"]
                ))
                # Spanish uses the same established word "Chats"; "vCard" is a format
                # name that stays unchanged across locales.
                if locale != "en" and (locale, key) != ("es", "desktop-chats") and key != "desktop-contact-vcard":
                    self.assertNotEqual(variants, en[key], (locale, key))

    def test_converter_preserves_manual_overlay_and_generated_bytes(self):
        locales = converter.ROOT / "desktop/locales"
        before = {path: path.read_bytes() for path in locales.rglob("*.ftl")}
        manifest_before = (locales / "manifest.json").read_bytes()
        subprocess.run([sys.executable, str(converter.ROOT / "desktop/tools/convert_locales.py")], check=True, cwd=converter.ROOT)
        subprocess.run([sys.executable, str(converter.ROOT / "desktop/tools/convert_locales.py"), "--check"], check=True, cwd=converter.ROOT)
        self.assertEqual(set(before), set(locales.rglob("*.ftl")))
        for path, content in before.items():
            self.assertEqual(path.read_bytes(), content, path)
        self.assertEqual((locales / "manifest.json").read_bytes(), manifest_before)

    def test_android_escaping(self):
        def decode(text):
            node = ET.Element("string")
            node.text = text
            return converter.android_text(node)
        self.assertEqual(decode(r'Couldn\'t load'), "Couldn't load")
        self.assertEqual(decode(r'\\ \" \n \t \u007B'), '\\ " \n \t {')
        self.assertEqual(decode('  alpha\n  beta  '), "alpha beta")
        self.assertEqual(decode('"  alpha  beta  "'), "  alpha  beta  ")
        self.assertEqual(decode(r'\@name \?attr'), "@name ?attr")
        with self.assertRaises(ValueError):
            decode("bad\\")
        with self.assertRaises(ValueError):
            decode('"unclosed')

    def test_printf_indices_types_and_literal_percent(self):
        self.assertEqual(converter.signature("%2$s %1$d %s %d %% %1$d"), [(1, "d"), (1, "d"), (1, "s"), (2, "d"), (2, "s")])
        self.assertEqual(converter.pattern("100%%"), '{ "100" }{ "%" }')
        self.assertEqual(converter.signature("literal 100%"), [])
        for malformed in ("%1", "%1$z", "%2.3f", "%f", "%1$06d"):
            with self.assertRaises(ValueError):
                converter.tokens(malformed)

    def test_fluent_escaping_and_empty(self):
        for text in ('{ $evil } "\\"\n\t\r', "", "  literal  ", "name =\n*[other] injected", "日本語 العربية 🙂", "%2$s then %1$d %%"):
            self.assertEqual(converter.decode_pattern(converter.pattern(text)), converter.normalized_tokens(text))

    def test_source_defects_are_explicit_and_only_tokens_change(self):
        repairs = self.manifest["source_repairs"]
        self.assertEqual(len(repairs), 2)
        self.assertEqual({repair["key"] for repair in repairs}, set(converter.REPAIRS))
        for repair in repairs:
            self.assertEqual(repair["locale"], "zh")
            self.assertEqual(repair["original"].replace("%1", "%1$d"), repair["corrected"])
        inherited = self.manifest["inherited_nontranslatable"]
        self.assertEqual(len(inherited), 9)
        self.assertEqual({entry["key"] for entry in inherited}, {"feature-dialog-dialog_map_attribution"})
        self.assertEqual({entry["locale"] for entry in inherited}, set(converter.LOCALES) - {"en"})

    def test_standalone_rust_runtime_against_android(self):
        rustc = shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc")
        tools = converter.ROOT / "desktop/tools"
        vectors = []
        for module in converter.MODULES:
            res = converter.ROOT / module / "src/main/res"
            base = converter.read_xml(res / "values/strings.xml")
            for locale, directory in converter.LOCALES.items():
                translated = converter.read_xml(res / directory / "strings.xml")
                for name, original in base.items():
                    key = f"{module.replace('/', '-')}-{name}"
                    values = dict(translated.get(name, original)["values"])
                    if locale == "zh" and key in converter.REPAIRS:
                        values["value"] = converter.REPAIRS[key][1]
                    for count in ((None,) if "value" in values else COUNTS):
                        text = values["value"] if count is None else values.get(category(locale, count), values["other"])
                        expected = "".join(value if kind == "text" else ARGUMENTS[value] for kind, value in converter.tokens(text))
                        quantity = "None" if count is None else f"Some({count})"
                        vectors.append(f"({rust_string(locale)}, {rust_string(key)}, {quantity}, {rust_string(expected)}),")
        self.assertEqual(len(vectors), 18610)
        for locale in converter.LOCALES:
            path = converter.ROOT / "desktop/locales" / locale / "desktop-extra.ftl"
            extra = converter.parse_ftl(path.read_text(encoding="utf-8"))
            for key, variants in extra.items():
                expected = "".join(
                    value if kind == "text" else ARGUMENTS[value]
                    for kind, value in variants["value"]
                )
                vectors.append(f"({rust_string(locale)}, {rust_string(key)}, None, {rust_string(expected)}),")
        self.assertEqual(len(vectors), 19300)
        with tempfile.TemporaryDirectory(prefix=".i18n-test-", dir=tools) as directory:
            directory = Path(directory)
            vector_file = directory / "vectors.rs"
            vector_file.write_text("&[\n" + "\n".join(vectors) + "\n]\n", encoding="utf-8")
            executable = directory / "i18n-tests"
            env = dict(os.environ, MONOGRAM_I18N_VECTORS=str(vector_file))
            subprocess.run([rustc, "--edition=2024", "--test", "-D", "warnings", str(tools / "i18n_tests.rs"), "-o", str(executable)], check=True, env=env, cwd=converter.ROOT)
            subprocess.run([str(executable)], check=True, cwd=converter.ROOT)
        print("Rust runtime: 18610 Android-derived and 690 desktop rendering vectors checked offline", flush=True)


if __name__ == "__main__":
    unittest.main(verbosity=2)
