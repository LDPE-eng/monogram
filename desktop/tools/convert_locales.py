#!/usr/bin/env python3
"""Convert the pinned Android catalog using only the Python standard library."""

import argparse
import collections
import hashlib
import json
from pathlib import Path
import re
import sys
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
MODULES = (
    "app", "core/ui", "feature/auth", "feature/chats", "feature/dialog",
    "feature/folders", "feature/profile", "feature/settings",
)
LOCALES = {
    "en": "values", "ru": "values-ru", "es": "values-es", "hy": "values-hy",
    "ja": "values-ja", "pt-BR": "values-pt-rBR", "sk": "values-sk",
    "tr": "values-tr", "uk": "values-uk", "zh": "values-zh",
}
QUANTITIES = ("zero", "one", "two", "few", "many", "other")
PRINTF = re.compile(r"%(?:(\d+)\$)?([ds%])")
REPAIRS = {
    "feature-dialog-dialog_instant_view_min_read": ("%1 分钟阅读", "%1$d 分钟阅读"),
    "feature-dialog-dialog_instant_view_chars": ("%1 个字符", "%1$d 个字符"),
}


def android_text(node):
    if list(node):
        raise ValueError("Inline XML markup requires an explicit conversion policy")
    text = node.text or ""
    escapes = {"n": "\n", "t": "\t", "r": "\r", "'": "'", '"': '"', "\\": "\\", "@": "@", "?": "?"}
    out = []
    i = 0
    quoted = False
    pending_space = False
    while i < len(text):
        c = text[i]
        if c == "\\":
            i += 1
            if i == len(text):
                raise ValueError("Trailing Android escape")
            c = text[i]
            if c == "u":
                digits = text[i + 1:i + 5]
                if not re.fullmatch(r"[0-9a-fA-F]{4}", digits):
                    raise ValueError("Invalid Android Unicode escape")
                c = chr(int(digits, 16))
                i += 4
            elif c in escapes:
                c = escapes[c]
            else:
                raise ValueError(f"Unsupported Android escape: {c!r}")
        elif c == '"':
            quoted = not quoted
            i += 1
            continue
        elif c.isspace() and not quoted:
            pending_space = bool(out)
            i += 1
            continue
        if pending_space:
            out.append(" ")
            pending_space = False
        out.append(c)
        i += 1
    if quoted:
        raise ValueError("Unbalanced Android quotation")
    return "".join(out)


def tokens(text):
    result = []
    position = 0
    implicit = 0
    for match in PRINTF.finditer(text):
        if match.start() > position:
            result.append(("text", text[position:match.start()]))
        index, kind = match.groups()
        if kind == "%":
            if index is not None:
                raise ValueError("Indexed literal percent")
            result.append(("text", "%"))
        else:
            if index is None:
                implicit += 1
                index = implicit
            result.append((kind, int(index)))
        position = match.end()
    if position < len(text):
        result.append(("text", text[position:]))
    # Refuse new printf forms rather than silently flattening their formatting.
    for kind, value in result:
        if kind == "text" and re.search(r"%[a-zA-Z0-9]", value):
            raise ValueError(f"Unsupported or malformed printf placeholder: {value!r}")
    return result


def signature(text):
    return sorted((index, kind) for kind, index in tokens(text) if kind != "text")


def fluent_literal(text):
    out = []
    for char in text:
        if char in ('"', "\\"):
            out.append("\\" + char)
        elif ord(char) < 32 or ord(char) == 127:
            out.append(f"\\u{ord(char):04X}")
        else:
            out.append(char)
    return '{ "' + "".join(out) + '" }'


def pattern(text):
    return "".join(
        fluent_literal(value) if kind == "text" else f"{{ $arg{value} }}"
        for kind, value in tokens(text)
    ) or fluent_literal("")


def decode_pattern(value):
    """Validate and round-trip the exact Fluent subset emitted above."""
    result = []
    while value:
        if value.startswith('{ "'):
            i = 3
            out = []
            while i < len(value) and value[i] != '"':
                char = value[i]
                i += 1
                if char == "\\":
                    char = value[i]
                    i += 1
                    if char == "u":
                        char = chr(int(value[i:i + 4], 16))
                        i += 4
                    elif char not in ('"', "\\"):
                        raise ValueError("Invalid Fluent escape")
                out.append(char)
            if value[i:i + 3] != '" }':
                raise ValueError("Invalid Fluent string literal")
            literal = "".join(out)
            if result and result[-1][0] == "text":
                result[-1] = ("text", result[-1][1] + literal)
            else:
                result.append(("text", literal))
            value = value[i + 3:]
        else:
            match = re.match(r"\{ \$arg([1-9]\d*) \}", value)
            if not match:
                raise ValueError(f"Invalid generated Fluent expression: {value!r}")
            result.append(("variable", int(match[1])))
            value = value[match.end():]
    return result


def normalized_tokens(text):
    result = []
    for kind, value in tokens(text):
        kind = "text" if kind == "text" else "variable"
        if kind == "text" and result and result[-1][0] == "text":
            result[-1] = (kind, result[-1][1] + value)
        else:
            result.append((kind, value))
    return result or [("text", "")]


def read_xml(path):
    result = {}
    for node in ET.parse(path).getroot():
        name = node.attrib["name"]
        if name in result or node.tag not in ("string", "plurals"):
            raise ValueError(f"Duplicate or unsupported resource: {path}:{name}")
        if node.tag == "string":
            values = {"value": android_text(node)}
        else:
            values = {}
            for item in node:
                quantity = item.attrib["quantity"]
                if item.tag != "item" or quantity not in QUANTITIES or quantity in values:
                    raise ValueError(f"Invalid quantity: {path}:{name}:{quantity}")
                values[quantity] = android_text(item)
            if "other" not in values:
                raise ValueError(f"Missing other quantity: {path}:{name}")
        result[name] = {
            "kind": node.tag, "translatable": node.attrib.get("translatable") != "false",
            "values": values,
        }
    return result


def parse_ftl(text):
    result = {}
    current = None
    for line in text.splitlines():
        if not line or line.startswith("#"):
            continue
        if line == "    }":
            current = None
            continue
        if line.startswith("    "):
            match = re.fullmatch(r"    (\*?)\[([a-z]+)\] (.*)", line)
            if current is None or not match:
                raise ValueError(f"Invalid variant: {line!r}")
            quantity = match[2]
            if quantity in result[current] or (match[1] == "*") != (quantity == "other"):
                raise ValueError("Duplicate quantity or incorrect default")
            result[current][quantity] = decode_pattern(match[3])
        else:
            key, value = line.split(" = ", 1)
            if not re.fullmatch(r"[a-zA-Z][a-zA-Z0-9_-]*", key) or key in result:
                raise ValueError(f"Invalid/duplicate Fluent key: {key}")
            if value == "{ $count ->":
                result[key] = {}
                current = key
            else:
                result[key] = {"value": decode_pattern(value)}
    return result


def generate(root=ROOT):
    catalogs = {locale: {} for locale in LOCALES}
    metadata = {}
    sources = []
    repairs = []
    inherited = []
    totals = collections.Counter()
    for module in MODULES:
        res = root / module / "src/main/res"
        base = read_xml(res / "values/strings.xml")
        totals.update(entry["kind"] for entry in base.values())
        prefix = module.replace("/", "-")
        for locale, directory in LOCALES.items():
            path = res / directory / "strings.xml"
            sources.append({"path": path.relative_to(root).as_posix(), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
            translated = read_xml(path)
            missing = base.keys() - translated.keys()
            if translated.keys() - base.keys() or any(base[name]["translatable"] for name in missing):
                raise ValueError(f"Locale key mismatch: {path}")
            for name, original in sorted(base.items()):
                key = f"{prefix}-{name}"
                entry = translated.get(name, original)
                values = dict(entry["values"])
                if name in missing:
                    inherited.append({"locale": locale, "key": key, "reason": "Android translatable=false", "source": f"{module}/src/main/res/values/strings.xml"})
                if locale == "zh" and key in REPAIRS:
                    before, after = REPAIRS[key]
                    if values != {"value": before}:
                        raise ValueError(f"Known zh defect changed; review repair: {key}")
                    values["value"] = after
                    repairs.append({"locale": locale, "key": key, "source": path.relative_to(root).as_posix(), "original": before, "corrected": after, "reason": "Restore missing $d in existing Android printf placeholder; no translated words changed"})
                if entry["kind"] != original["kind"]:
                    raise ValueError(f"Resource kind mismatch: {locale}:{key}")
                base_values = original["values"]
                for quantity, text in values.items():
                    reference = base_values.get(quantity, base_values.get("other"))
                    if reference is None or signature(text) != signature(reference):
                        raise ValueError(f"Placeholder signature mismatch: {locale}:{key}:{quantity}")
                    if decode_pattern(pattern(text)) != normalized_tokens(text):
                        raise ValueError(f"Escaping round-trip mismatch: {locale}:{key}:{quantity}")
                catalogs[locale][key] = values
                if key not in metadata:
                    metadata[key] = {"kind": original["kind"], "source_module": module, "translatable": original["translatable"], "locales": {}}
                metadata[key]["locales"][locale] = {
                    "quantities": list(values) if entry["kind"] == "plurals" else [],
                    "signatures": {q: [{"variable": f"arg{i}", "printf_type": kind} for i, kind in signature(text)] for q, text in values.items()},
                }
    if totals != {"string": 1317, "plurals": 32} or len(sources) != 80 or len(repairs) != 2:
        raise ValueError(f"Pinned catalog changed: {totals}, {len(sources)} files, {len(repairs)} repairs")
    output = {}
    for locale, catalog in catalogs.items():
        lines = ["# Generated by desktop/tools/convert_locales.py; do not edit.", "# $argN retains the Android printf argument index; $count selects quantity.", "# Literal placeables preserve Android text, whitespace and Fluent metacharacters."]
        for key, values in sorted(catalog.items()):
            if locale == "zh" and key in REPAIRS:
                lines.append("# Known Android printf defect repaired; see ../manifest.json source_repairs.")
            if "value" in values:
                lines.append(f"{key} = {pattern(values['value'])}")
            else:
                lines.append(f"{key} = {{ $count ->")
                for quantity in QUANTITIES:
                    if quantity in values:
                        default = "*" if quantity == "other" else ""
                        lines.append(f"    {default}[{quantity}] {pattern(values[quantity])}")
                lines.append("    }")
        text = "\n".join(lines) + "\n"
        expected = {k: {q: normalized_tokens(t) for q, t in v.items()} for k, v in catalog.items()}
        if parse_ftl(text) != expected:
            raise ValueError(f"Generated Fluent round-trip failed: {locale}")
        output[f"{locale}/monogram.ftl"] = text.encode()
    manifest = {
        "schema": 1, "source_revision": "04ce208ed1cd49067226f3f28e4adef47f779114",
        "counts": {"xml_files": len(sources), "base_strings": totals["string"], "base_plurals": totals["plurals"], "locales": len(LOCALES), "keys_per_locale": len(metadata), "source_strings_all_locales": totals["string"] * len(LOCALES) - len(inherited), "source_plurals_all_locales": totals["plurals"] * len(LOCALES)},
        "sources": sources, "source_repairs": repairs, "inherited_nontranslatable": inherited,
        "resources": {name: hashlib.sha256(content).hexdigest() for name, content in sorted(output.items())},
        "entries": metadata,
    }
    output["manifest.json"] = (json.dumps(manifest, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode()
    return output, manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Verify checked-in output without writing")
    args = parser.parse_args()
    output, manifest = generate()
    destination = ROOT / "desktop/locales"
    stale = []
    expected_ftl = {destination / name for name in output if name.endswith(".ftl")}
    manual_ftl = {destination / locale / "desktop-extra.ftl" for locale in LOCALES}
    stale.extend(str(path.relative_to(ROOT)) for path in destination.glob("**/*.ftl") if path not in expected_ftl | manual_ftl)
    for name, content in output.items():
        path = destination / name
        if args.check:
            if not path.is_file() or path.read_bytes() != content:
                stale.append(path.relative_to(ROOT).as_posix())
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
    if stale:
        print("Stale/unexpected generated resources: " + ", ".join(stale), file=sys.stderr)
        return 1
    print(json.dumps(manifest["counts"], sort_keys=True))
    print("Verified: keys, printf signatures, quantity sets, decoded text round-trip; 2 explicit zh repairs, 9 inherited nontranslatable entries.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
