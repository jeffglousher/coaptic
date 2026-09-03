#!/usr/bin/env python3
"""Deterministic OKF frontmatter extract (pass 1) and validate (pass 3).

Stdlib only. No network. No LLM. Run from the repo root:

    python3 .agents/skills/okf-frontmatter/scripts/extract_frontmatter.py
    python3 .agents/skills/okf-frontmatter/scripts/extract_frontmatter.py --check
    python3 .agents/skills/okf-frontmatter/scripts/extract_frontmatter.py --validate
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tomllib
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


RESERVED_NAMES = frozenset({"index.md", "log.md"})
CORE_RFCS = frozenset({7252, 7641, 7959, 9175, 9177})
DETERMINISTIC_BY = "process:okf-frontmatter"
RFC_STEM_RE = re.compile(r"^rfc(\d+)$")
MONTHS = (
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
)
MONTH_RE = re.compile(r"\b(" + "|".join(MONTHS) + r")\s+(\d{4})\b")
RFC_NUM_RE = re.compile(r"^Request for Comments:\s+(\d+)\b")
UPDATES_RE = re.compile(r"^Updates:\s+([0-9, ]+?)(?:\s{2,}|\s*$)")
UPDATED_BY_RE = re.compile(r"^Updated by:\s+([0-9, ]+?)(?:\s{2,}|\s*$)", re.I)
CATEGORY_RE = re.compile(
    r"^Category:\s+(Standards Track|Informational|Experimental|Best Current Practice)\b"
)
TD_KEY_RE = re.compile(r"^TD_[A-Za-z0-9_]+$")
HUMAN_ACTOR_RE = re.compile(r"^human:")
ON_KEY_RE = re.compile(r"^(\s*)([A-Za-z_][\w]*)\s*:")

PATH_TYPES = {
    "crate": "Crate",
    "architecture": "Architecture",
    "memory-areas": "Memory Areas",
    "ci": "Playbook",
    "review": "Policy",
    "okf-frontmatter": "Playbook",
    "plugtest/requirements": "Playbook",
    "plugtest/base": "Test Descriptions",
    "plugtest/block": "Test Descriptions",
    "plugtest/link": "Test Descriptions",
    "plugtest/dtls": "Test Descriptions",
    "plugtest/6lowpan": "Test Descriptions",
    "plugtest/td-coap4/README": "Reference",
}

PLUGTEST_YML = {
    "plugtest/base": "td-coap4/base.yml",
    "plugtest/block": "td-coap4/block.yml",
    "plugtest/link": "td-coap4/link.yml",
    "plugtest/dtls": "td-coap4/dtls.yml",
    "plugtest/6lowpan": "td-coap4/6lowpan.yml",
}

# Producer + OKF v0.2 keys. Extra keys are allowed (warn only).
KNOWN_KEYS = frozenset(
    {
        "type",
        "title",
        "description",
        "resource",
        "tags",
        "sources",
        "generated",
        "verified",
        "status",
        "stale_after",
        "deterministic",
        "ietf_status",
        "rfc_number",
        "date",
        "txt_bytes",
        "pdf_bytes",
        "updates",
        "updated_by",
        "crate_name",
        "license",
        "edition",
        "rust-version",
        "features",
        "td_ids",
        "ci_triggers",
        "release_triggers",
        "scope",
        "okf_version",
    }
)

COMPARE_FIELDS = (
    "type",
    "title",
    "resource",
    "ietf_status",
    "rfc_number",
    "date",
    "txt_bytes",
    "pdf_bytes",
    "updates",
    "updated_by",
    "crate_name",
    "license",
    "edition",
    "rust-version",
    "features",
    "status",
    "ci_triggers",
    "release_triggers",
)


def repo_root() -> Path:
    here = Path(__file__).resolve()
    for parent in here.parents:
        if (parent / "knowledge").is_dir() and (parent / "Cargo.toml").is_file():
            return parent
    return Path.cwd()


def utc_now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def concept_id(md_path: Path, knowledge: Path) -> str:
    return md_path.relative_to(knowledge).with_suffix("").as_posix()


def iter_concept_paths(knowledge: Path) -> list[Path]:
    paths = []
    for path in sorted(knowledge.rglob("*.md")):
        if path.name in RESERVED_NAMES:
            continue
        paths.append(path)
    return paths


def stamp(fields: list[str], at: str) -> dict[str, Any]:
    return {"by": DETERMINISTIC_BY, "at": at, "fields": list(fields)}


def set_field(fm: dict[str, Any], fields: list[str], key: str, value: Any) -> None:
    fm[key] = value
    if key not in fields:
        fields.append(key)


def parse_rfc_header(txt_path: Path) -> dict[str, Any]:
    """Parse the IETF plain-text header (first ~80 lines). Omit unparsed fields."""
    lines = txt_path.read_text(encoding="utf-8", errors="replace").splitlines()[:80]
    out: dict[str, Any] = {}
    abstract_idx = next((i for i, ln in enumerate(lines) if ln.strip() == "Abstract"), None)
    header_lines = lines[:abstract_idx] if abstract_idx is not None else lines

    for line in header_lines:
        m = RFC_NUM_RE.match(line)
        if m:
            out["header_rfc_number"] = int(m.group(1))
            continue
        m = UPDATES_RE.match(line)
        if m:
            nums = [n.strip() for n in m.group(1).split(",") if n.strip()]
            if nums:
                out["updates"] = nums
            continue
        m = UPDATED_BY_RE.match(line)
        if m:
            nums = [n.strip() for n in m.group(1).split(",") if n.strip()]
            if nums:
                out["updated_by"] = nums
            continue
        m = CATEGORY_RE.match(line)
        if m:
            category = m.group(1)
            window = "\n".join(header_lines)
            if re.search(r"(?m)^\s*Proposed Standard\s*$", window) or re.search(
                r"\bStatus:\s*Proposed Standard\b", window
            ):
                out["ietf_status"] = "Proposed Standard"
            elif re.search(r"(?m)^\s*Internet Standard\s*$", window) or re.search(
                r"\bStatus:\s*Internet Standard\b", window
            ):
                # Avoid matching "Internet Standards Track".
                if "Internet Standards Track" not in window or re.search(
                    r"(?m)^\s*Internet Standard\s*$", window
                ):
                    out["ietf_status"] = "Internet Standard"
                else:
                    out["ietf_status"] = category
            else:
                out["ietf_status"] = category

    dates: list[str] = []
    for line in header_lines:
        for m in MONTH_RE.finditer(line):
            if line[m.end() :].strip() == "":
                dates.append(f"{m.group(1)} {m.group(2)}")
    if dates:
        out["date"] = dates[-1]

    if abstract_idx is not None:
        i = abstract_idx - 1
        while i >= 0 and not lines[i].strip():
            i -= 1
        end = i
        while i >= 0 and lines[i].strip():
            i -= 1
        title_lines = [ln.strip() for ln in lines[i + 1 : end + 1] if ln.strip()]
        if title_lines:
            out["title"] = " ".join(title_lines)
    return out


def parse_cargo(toml_path: Path) -> dict[str, Any]:
    data = tomllib.loads(toml_path.read_text(encoding="utf-8"))
    pkg = data.get("package") or {}
    out: dict[str, Any] = {}
    if "name" in pkg:
        out["crate_name"] = pkg["name"]
    if "license" in pkg:
        out["license"] = pkg["license"]
    if "edition" in pkg:
        out["edition"] = str(pkg["edition"])
    if "rust-version" in pkg:
        out["rust-version"] = str(pkg["rust-version"])
    if "description" in pkg:
        out["description"] = pkg["description"]
    features = data.get("features")
    if isinstance(features, dict):
        out["features"] = {k: list(v) for k, v in features.items()}
    return out


def parse_workflow_triggers(text: str) -> list[str]:
    """Read on.* keys. No invented jobs."""
    lines = text.splitlines()
    i = 0
    while i < len(lines):
        if re.match(r"^on:\s*$", lines[i]):
            i += 1
            break
        i += 1
    else:
        return []

    triggers: list[str] = []
    top_indent: int | None = None
    while i < len(lines):
        line = lines[i]
        if line.strip() == "" or line.lstrip().startswith("#"):
            i += 1
            continue
        if line[0] not in " \t":
            break
        m = ON_KEY_RE.match(line)
        if not m:
            i += 1
            continue
        indent = len(m.group(1))
        if top_indent is None:
            top_indent = indent
        if indent > top_indent:
            i += 1
            continue
        key = m.group(2)
        if key == "push":
            has_tags = False
            j = i + 1
            while j < len(lines):
                nxt = lines[j]
                if nxt.strip() == "" or nxt.lstrip().startswith("#"):
                    j += 1
                    continue
                if nxt[0] not in " \t":
                    break
                nm = ON_KEY_RE.match(nxt)
                if nm and len(nm.group(1)) <= indent:
                    break
                if nm and nm.group(2) == "tags":
                    has_tags = True
                    break
                j += 1
            triggers.append("on.push.tags" if has_tags else "on.push")
        elif key == "pull_request":
            triggers.append("on.pull_request")
        elif key == "workflow_dispatch":
            triggers.append("workflow_dispatch")
        else:
            triggers.append(f"on.{key}")
        i += 1
    return triggers


def parse_td_ids(yml_path: Path) -> list[str]:
    ids: list[str] = []
    seen: set[str] = set()
    for line in yml_path.read_text(encoding="utf-8", errors="replace").splitlines():
        if line.startswith(" ") or line.startswith("\t") or line.lstrip().startswith("#"):
            continue
        if ":" not in line:
            continue
        key = line.split(":", 1)[0].strip()
        if TD_KEY_RE.match(key) and key not in seen:
            seen.add(key)
            ids.append(key)
    return ids


def parse_td_objectives(yml_path: Path) -> list[tuple[str, str]]:
    """Pass-2 helper: TD id + YAML obj (not used by pass 1)."""
    text = yml_path.read_text(encoding="utf-8", errors="replace")
    rows: list[tuple[str, str]] = []
    current: str | None = None
    for line in text.splitlines():
        if line.startswith(" ") or line.startswith("\t"):
            if current and re.match(r"^\s+obj:\s+", line):
                obj = line.split(":", 1)[1].strip()
                if (obj.startswith('"') and obj.endswith('"')) or (
                    obj.startswith("'") and obj.endswith("'")
                ):
                    obj = obj[1:-1]
                rows.append((current, obj))
                current = None
            continue
        if ":" not in line or line.lstrip().startswith("#"):
            continue
        key = line.split(":", 1)[0].strip()
        if TD_KEY_RE.match(key):
            current = key
    return rows


def extract_rfc(md_path: Path, fields: list[str], at: str) -> dict[str, Any]:
    stem = md_path.stem
    m = RFC_STEM_RE.match(stem)
    fm: dict[str, Any] = {}
    set_field(fm, fields, "type", "RFC")
    if not m:
        fm["deterministic"] = stamp(fields, at)
        return fm
    number = int(m.group(1))
    set_field(fm, fields, "rfc_number", number)
    resource = f"{stem}.txt"
    set_field(fm, fields, "resource", resource)
    txt = md_path.with_name(resource)
    pdf = md_path.with_name(f"{stem}.pdf")
    if txt.is_file():
        header = parse_rfc_header(txt)
        if "title" in header:
            set_field(fm, fields, "title", header["title"])
        if "ietf_status" in header:
            set_field(fm, fields, "ietf_status", header["ietf_status"])
        if "date" in header:
            set_field(fm, fields, "date", header["date"])
        if "updates" in header:
            set_field(fm, fields, "updates", header["updates"])
        if "updated_by" in header:
            set_field(fm, fields, "updated_by", header["updated_by"])
        set_field(fm, fields, "txt_bytes", txt.stat().st_size)
    if pdf.is_file():
        set_field(fm, fields, "pdf_bytes", pdf.stat().st_size)
    tags = ["rfc", str(number), "core" if number in CORE_RFCS else "related"]
    set_field(fm, fields, "tags", tags)
    set_field(fm, fields, "status", "stable")
    set_field(
        fm,
        fields,
        "sources",
        [
            {
                "id": "rfc-editor-txt",
                "resource": f"https://www.rfc-editor.org/rfc/{stem}.txt",
            },
            {
                "id": "rfc-editor-pdf",
                "resource": f"https://www.rfc-editor.org/rfc/{stem}.pdf",
            },
        ],
    )
    fm["deterministic"] = stamp(fields, at)
    return fm


def extract_concept(md_path: Path, root: Path, at: str) -> dict[str, Any]:
    knowledge = root / "knowledge"
    cid = concept_id(md_path, knowledge)
    fields: list[str] = []
    rfc_m = RFC_STEM_RE.match(md_path.stem)
    if cid.startswith("rfcs/") and rfc_m:
        return extract_rfc(md_path, fields, at)

    fm: dict[str, Any] = {}
    path_type = PATH_TYPES.get(cid)
    if cid.startswith("rfcs/") and rfc_m:
        path_type = "RFC"
    if path_type:
        set_field(fm, fields, "type", path_type)

    if cid == "crate":
        cargo = root / "Cargo.toml"
        if cargo.is_file():
            set_field(fm, fields, "resource", "../Cargo.toml")
            parsed = parse_cargo(cargo)
            if "crate_name" in parsed:
                set_field(fm, fields, "title", parsed["crate_name"])
                set_field(fm, fields, "crate_name", parsed["crate_name"])
            for key in ("license", "edition", "rust-version", "description", "features"):
                if key in parsed:
                    set_field(fm, fields, key, parsed[key])
    elif cid in {"architecture", "memory-areas"}:
        if (root / "design.md").is_file():
            set_field(fm, fields, "resource", "../design.md")
    elif cid == "ci":
        ci = root / ".github" / "workflows" / "ci.yml"
        release = root / ".github" / "workflows" / "release.yml"
        if ci.is_file():
            set_field(fm, fields, "resource", "../.github/workflows/ci.yml")
            set_field(fm, fields, "ci_triggers", parse_workflow_triggers(ci.read_text(encoding="utf-8")))
        if release.is_file():
            set_field(
                fm,
                fields,
                "release_triggers",
                parse_workflow_triggers(release.read_text(encoding="utf-8")),
            )
    elif cid == "review":
        if (root / "AGENTS.md").is_file():
            set_field(fm, fields, "resource", "../AGENTS.md")
    elif cid == "okf-frontmatter":
        script = Path(".agents/skills/okf-frontmatter/scripts/extract_frontmatter.py")
        if (root / script).is_file():
            set_field(fm, fields, "resource", f"../{script.as_posix()}")
    elif cid in PLUGTEST_YML:
        rel = PLUGTEST_YML[cid]
        yml = md_path.parent / rel
        if yml.is_file():
            set_field(fm, fields, "resource", rel)
            set_field(fm, fields, "td_ids", parse_td_ids(yml))

    if fields:
        fm["deterministic"] = stamp(fields, at)
    return fm


def extract_all(root: Path, at: str | None = None) -> dict[str, dict[str, Any]]:
    when = at or utc_now()
    knowledge = root / "knowledge"
    mapping: dict[str, dict[str, Any]] = {}
    for path in iter_concept_paths(knowledge):
        mapping[concept_id(path, knowledge)] = extract_concept(path, root, when)
    return mapping


# --- YAML subset (dump + load) ------------------------------------------------


def _needs_quote(s: str) -> bool:
    if s == "" or s.strip() != s:
        return True
    if s in {"true", "false", "null", "True", "False", "None"}:
        return True
    if re.fullmatch(r"-?\d+(?:\.\d+)?", s):
        return True
    if any(ch in s for ch in ":#{}[],&*!|>%@`'\\\""):
        return True
    if s.startswith(("-", "?", ":")):
        return True
    return False


def format_scalar(value: Any) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int) and not isinstance(value, bool):
        return str(value)
    if value is None:
        return "null"
    s = str(value)
    if _needs_quote(s):
        return json.dumps(s, ensure_ascii=False)
    return s


def dump_yaml(value: Any, indent: int = 0) -> str:
    pad = "  " * indent
    if isinstance(value, dict):
        if not value:
            return "{}"
        parts = []
        for key, item in value.items():
            key_s = format_scalar(key) if _needs_quote(str(key)) else str(key)
            if isinstance(item, dict):
                if not item:
                    parts.append(f"{pad}{key_s}: {{}}")
                else:
                    parts.append(f"{pad}{key_s}:")
                    parts.append(dump_yaml(item, indent + 1))
            elif isinstance(item, list):
                if not item:
                    parts.append(f"{pad}{key_s}: []")
                elif all(not isinstance(x, (dict, list)) for x in item):
                    inner = ", ".join(format_scalar(x) for x in item)
                    parts.append(f"{pad}{key_s}: [{inner}]")
                else:
                    parts.append(f"{pad}{key_s}:")
                    parts.append(dump_yaml(item, indent + 1))
            else:
                parts.append(f"{pad}{key_s}: {format_scalar(item)}")
        return "\n".join(parts)
    if isinstance(value, list):
        if not value:
            return f"{pad}[]"
        parts = []
        for item in value:
            if isinstance(item, dict):
                if not item:
                    parts.append(f"{pad}- {{}}")
                    continue
                first = True
                for key, sub in item.items():
                    key_s = format_scalar(key) if _needs_quote(str(key)) else str(key)
                    if first:
                        prefix = f"{pad}- "
                        first = False
                    else:
                        prefix = f"{pad}  "
                    if isinstance(sub, (dict, list)) and sub:
                        parts.append(f"{prefix}{key_s}:")
                        parts.append(dump_yaml(sub, indent + 2 if prefix.endswith("- ") else indent + 2))
                    elif isinstance(sub, list) and not sub:
                        parts.append(f"{prefix}{key_s}: []")
                    elif isinstance(sub, dict) and not sub:
                        parts.append(f"{prefix}{key_s}: {{}}")
                    else:
                        parts.append(f"{prefix}{key_s}: {format_scalar(sub)}")
            elif isinstance(item, list):
                parts.append(f"{pad}-")
                parts.append(dump_yaml(item, indent + 1))
            else:
                parts.append(f"{pad}- {format_scalar(item)}")
        return "\n".join(parts)
    return f"{pad}{format_scalar(value)}"


def dump_mapping(mapping: dict[str, Any]) -> str:
    return dump_yaml(mapping) + "\n"


def _parse_quoted(s: str, i: int) -> tuple[str, int]:
    quote = s[i]
    i += 1
    out = []
    while i < len(s):
        ch = s[i]
        if ch == "\\" and i + 1 < len(s):
            out.append(s[i + 1])
            i += 2
            continue
        if ch == quote:
            return "".join(out), i + 1
        out.append(ch)
        i += 1
    raise ValueError(f"unterminated string: {s!r}")


def _parse_inline_value(s: str, i: int) -> tuple[Any, int]:
    while i < len(s) and s[i] == " ":
        i += 1
    if i >= len(s):
        return "", i
    if s[i] in "\"'":
        return _parse_quoted(s, i)
    if s[i] == "{":
        return _parse_inline_map(s, i)
    if s[i] == "[":
        return _parse_inline_list(s, i)
    j = i
    while j < len(s) and s[j] not in ",}]":
        j += 1
    raw = s[i:j].strip()
    return _scalar(raw), j


def _parse_inline_key(s: str, i: int) -> tuple[str, int]:
    while i < len(s) and s[i] == " ":
        i += 1
    if i < len(s) and s[i] in "\"'":
        key, i = _parse_quoted(s, i)
        return str(key), i
    j = i
    while j < len(s) and s[j] not in ":{},[]":
        j += 1
    return s[i:j].strip(), j


def _parse_inline_map(s: str, i: int) -> tuple[dict[str, Any], int]:
    assert s[i] == "{"
    i += 1
    out: dict[str, Any] = {}
    while i < len(s):
        while i < len(s) and s[i] in " \t":
            i += 1
        if i < len(s) and s[i] == "}":
            return out, i + 1
        key, i = _parse_inline_key(s, i)
        while i < len(s) and s[i] in " \t":
            i += 1
        if i >= len(s) or s[i] != ":":
            raise ValueError(f"expected ':' in inline map: {s!r}")
        i += 1
        val, i = _parse_inline_value(s, i)
        out[str(key)] = val
        while i < len(s) and s[i] in " \t":
            i += 1
        if i < len(s) and s[i] == ",":
            i += 1
            continue
        if i < len(s) and s[i] == "}":
            return out, i + 1
        raise ValueError(f"bad inline map: {s!r}")
    raise ValueError(f"unterminated inline map: {s!r}")


def _parse_inline_list(s: str, i: int) -> tuple[list[Any], int]:
    assert s[i] == "["
    i += 1
    out: list[Any] = []
    while i < len(s):
        while i < len(s) and s[i] in " \t":
            i += 1
        if i < len(s) and s[i] == "]":
            return out, i + 1
        val, i = _parse_inline_value(s, i)
        out.append(val)
        while i < len(s) and s[i] in " \t":
            i += 1
        if i < len(s) and s[i] == ",":
            i += 1
            continue
        if i < len(s) and s[i] == "]":
            return out, i + 1
        raise ValueError(f"bad inline list: {s!r}")
    raise ValueError(f"unterminated inline list: {s!r}")


def _scalar(raw: str) -> Any:
    if raw in {"true", "True"}:
        return True
    if raw in {"false", "False"}:
        return False
    if raw in {"null", "None", "~"}:
        return None
    if re.fullmatch(r"-?\d+", raw):
        return int(raw)
    return raw


def parse_yaml_value(raw: str) -> Any:
    s = raw.strip()
    if not s:
        return ""
    if s[0] in "\"'":
        val, end = _parse_quoted(s, 0)
        if end == len(s):
            return val
    if s[0] == "{" and s[-1] == "}":
        val, end = _parse_inline_map(s, 0)
        if end == len(s):
            return val
    if s[0] == "[" and s[-1] == "]":
        val, end = _parse_inline_list(s, 0)
        if end == len(s):
            return val
    return _scalar(s)


def _line_indent(line: str) -> int | None:
    if not line.strip() or line.lstrip().startswith("#"):
        return None
    return len(line) - len(line.lstrip(" "))


def _split_key(stripped: str) -> tuple[str, str | None]:
    if stripped[0] in "\"'":
        key, i = _parse_quoted(stripped, 0)
        rest = stripped[i:].lstrip()
        if not rest.startswith(":"):
            raise ValueError(f"expected ':' after quoted key: {stripped!r}")
        rest = rest[1:]
        return key, rest.lstrip() if rest.strip() else None
    m = re.match(r"^([A-Za-z_][\w.-]*)\s*:(?:\s+(.*))?$", stripped)
    if not m:
        raise ValueError(f"expected key: {stripped!r}")
    rest = m.group(2)
    return m.group(1), rest if rest not in (None, "") else None


def _parse_block(lines: list[str], idx: int, indent: int) -> tuple[Any, int]:
    while idx < len(lines) and _line_indent(lines[idx]) is None:
        idx += 1
    if idx >= len(lines):
        return {}, idx
    cur = _line_indent(lines[idx])
    if cur is None or cur < indent:
        return {}, idx
    stripped = lines[idx].strip()
    if stripped.startswith("- ") or stripped == "-":
        return _parse_list(lines, idx, cur)
    return _parse_map(lines, idx, cur)


def _parse_map(lines: list[str], idx: int, indent: int) -> tuple[dict[str, Any], int]:
    result: dict[str, Any] = {}
    while idx < len(lines):
        ind = _line_indent(lines[idx])
        if ind is None:
            idx += 1
            continue
        if ind < indent:
            break
        if ind > indent:
            raise ValueError(f"bad indent at {lines[idx]!r}")
        stripped = lines[idx].strip()
        if stripped.startswith("-"):
            break
        key, rest = _split_key(stripped)
        idx += 1
        if rest is not None:
            result[key] = parse_yaml_value(rest)
            continue
        while idx < len(lines) and _line_indent(lines[idx]) is None:
            if lines[idx].strip() == "":
                idx += 1
                continue
            break
        if idx >= len(lines):
            result[key] = None
            break
        nxt = _line_indent(lines[idx])
        if nxt is None or nxt <= indent:
            result[key] = None
            continue
        value, idx = _parse_block(lines, idx, nxt)
        result[key] = value
    return result, idx


def _parse_list(lines: list[str], idx: int, indent: int) -> tuple[list[Any], int]:
    result: list[Any] = []
    while idx < len(lines):
        ind = _line_indent(lines[idx])
        if ind is None:
            idx += 1
            continue
        if ind < indent:
            break
        stripped = lines[idx].strip()
        if not (stripped.startswith("- ") or stripped == "-"):
            break
        if stripped == "-":
            rest = ""
        else:
            rest = stripped[2:]
        idx += 1
        if rest == "":
            while idx < len(lines) and _line_indent(lines[idx]) is None:
                idx += 1
            if idx >= len(lines):
                result.append(None)
                break
            nxt = _line_indent(lines[idx])
            if nxt is None or nxt <= indent:
                result.append(None)
                continue
            value, idx = _parse_block(lines, idx, nxt)
            result.append(value)
            continue
        if rest.lstrip().startswith("- ") or (
            ":" in rest and not rest.startswith(("'", '"', "{", "[")) and _looks_like_map_item(rest)
        ):
            # inline "- key: val" first field of a mapping
            first_key, first_rest = _split_key(rest)
            item: dict[str, Any] = {
                first_key: parse_yaml_value(first_rest) if first_rest is not None else None
            }
            while idx < len(lines):
                nind = _line_indent(lines[idx])
                if nind is None:
                    idx += 1
                    continue
                if nind <= indent:
                    break
                nstripped = lines[idx].strip()
                if nstripped.startswith("-"):
                    break
                nkey, nrest = _split_key(nstripped)
                idx += 1
                if nrest is not None:
                    item[nkey] = parse_yaml_value(nrest)
                    continue
                while idx < len(lines) and _line_indent(lines[idx]) is None:
                    idx += 1
                if idx >= len(lines):
                    item[nkey] = None
                    break
                nxt = _line_indent(lines[idx])
                if nxt is None or nxt <= nind:
                    item[nkey] = None
                    continue
                value, idx = _parse_block(lines, idx, nxt)
                item[nkey] = value
            result.append(item)
            continue
        result.append(parse_yaml_value(rest))
    return result, idx


def _looks_like_map_item(rest: str) -> bool:
    try:
        _split_key(rest)
        return True
    except ValueError:
        return False


def parse_yaml_mapping(text: str) -> dict[str, Any]:
    lines = text.splitlines()
    data, _ = _parse_map(lines, 0, 0)
    return data


def split_frontmatter(text: str) -> tuple[dict[str, Any] | None, str, bool]:
    """Return (mapping or None, body, has_delimiters)."""
    if not text.startswith("---"):
        first = text.splitlines()[0] if text else ""
        if first.strip() != "---":
            return None, text, False
    lines = text.splitlines(keepends=True)
    if not lines or lines[0].strip() != "---":
        return None, text, False
    end = None
    for i, line in enumerate(lines[1:], start=1):
        if line.strip() == "---":
            end = i
            break
    if end is None:
        return None, text, True
    fm_text = "".join(lines[1:end])
    body = "".join(lines[end + 1 :])
    try:
        return parse_yaml_mapping(fm_text), body, True
    except ValueError:
        return None, text, True


def load_frontmatter(path: Path) -> tuple[dict[str, Any] | None, str, bool]:
    return split_frontmatter(path.read_text(encoding="utf-8"))


FRONTMATTER_ORDER = (
    "type",
    "title",
    "description",
    "resource",
    "rfc_number",
    "ietf_status",
    "date",
    "txt_bytes",
    "pdf_bytes",
    "updates",
    "updated_by",
    "crate_name",
    "license",
    "edition",
    "rust-version",
    "features",
    "td_ids",
    "ci_triggers",
    "release_triggers",
    "tags",
    "status",
    "scope",
    "sources",
    "deterministic",
    "generated",
    "verified",
)


def order_frontmatter(fm: dict[str, Any]) -> dict[str, Any]:
    ordered: dict[str, Any] = {}
    for key in FRONTMATTER_ORDER:
        if key in fm:
            ordered[key] = fm[key]
    for key, value in fm.items():
        if key not in ordered:
            ordered[key] = value
    return ordered


def dump_frontmatter_file(fm: dict[str, Any], body: str) -> str:
    text = "---\n" + dump_yaml(order_frontmatter(fm)) + "\n---\n"
    if body and not body.startswith("\n") and not body.startswith("#"):
        # keep a single leading newline when the body starts with a heading
        pass
    if body.startswith("\n"):
        text += body
    elif body:
        text += "\n" + body
        if not body.endswith("\n"):
            text += "\n"
    else:
        text += "\n"
    return text


# --- Check / validate ---------------------------------------------------------


def _as_str_set(values: Any) -> set[str]:
    if not isinstance(values, list):
        return set()
    return {str(v) for v in values}


def _source_key(entry: Any) -> str | None:
    if not isinstance(entry, dict):
        return None
    if entry.get("id") is not None:
        return f"id:{entry['id']}"
    if entry.get("resource") is not None:
        return f"resource:{entry['resource']}"
    return None


def _human_actors(verified: Any) -> set[str]:
    if verified is None:
        return set()
    items = verified if isinstance(verified, list) else [verified]
    actors = set()
    for item in items:
        if isinstance(item, dict):
            by = item.get("by")
            if isinstance(by, str) and HUMAN_ACTOR_RE.match(by):
                actors.add(by)
    return actors


def _git_show(root: Path, rel: str) -> str | None:
    try:
        proc = subprocess.run(
            ["git", "show", f"origin/main:{rel}"],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError:
        return None
    if proc.returncode != 0:
        return None
    return proc.stdout


def main_human_verifiers(root: Path, rel: str) -> set[str]:
    text = _git_show(root, rel)
    if text is None:
        return set()
    fm, _, _ = split_frontmatter(text)
    if not fm:
        return set()
    return _human_actors(fm.get("verified"))


def values_equal(a: Any, b: Any) -> bool:
    if type(a) is type(b):
        return a == b
    if isinstance(a, (int, str)) and isinstance(b, (int, str)):
        return str(a) == str(b)
    return a == b


def check_rfc_concepts(root: Path) -> int:
    """Fail if an RFC concept title/resource disagrees with the sibling txt."""
    errors = 0
    knowledge = root / "knowledge" / "rfcs"
    if not knowledge.is_dir():
        print("error: knowledge/rfcs missing", file=sys.stderr)
        return 1
    for md in sorted(knowledge.glob("rfc*.md")):
        m = RFC_STEM_RE.match(md.stem)
        if not m:
            continue
        fm, _, has_fm = load_frontmatter(md)
        if not has_fm or fm is None:
            print(f"NO-GO {md}: missing frontmatter", file=sys.stderr)
            errors += 1
            continue
        txt = md.with_name(f"{md.stem}.txt")
        if not txt.is_file():
            print(f"NO-GO {md}: missing {txt.name}", file=sys.stderr)
            errors += 1
            continue
        header = parse_rfc_header(txt)
        expected_resource = f"{md.stem}.txt"
        if fm.get("resource") != expected_resource:
            print(
                f"NO-GO {md}: resource {fm.get('resource')!r} != {expected_resource!r}",
                file=sys.stderr,
            )
            errors += 1
        if "title" in header and fm.get("title") != header["title"]:
            print(
                f"NO-GO {md}: title {fm.get('title')!r} != txt {header['title']!r}",
                file=sys.stderr,
            )
            errors += 1
    if errors:
        print(f"--check failed: {errors} error(s)", file=sys.stderr)
        return 1
    print("ok: RFC title/resource match sibling txt")
    return 0


def validate(root: Path) -> int:
    """Pass 3: review finished files on disk. Critical → non-zero."""
    knowledge = root / "knowledge"
    fresh = extract_all(root)
    errors: list[str] = []
    warnings: list[str] = []

    for reserved in knowledge.rglob("*.md"):
        if reserved.name not in RESERVED_NAMES:
            continue
        fm, _, has_fm = load_frontmatter(reserved)
        rel = reserved.relative_to(root).as_posix()
        if not has_fm or fm is None:
            continue
        if "type" in fm:
            errors.append(f"{rel}: reserved file carries concept type {fm['type']!r}")
        if reserved == knowledge / "index.md":
            extra = set(fm) - {"okf_version"}
            if extra:
                warnings.append(f"{rel}: extra keys on bundle-root index: {sorted(extra)}")
        elif fm:
            warnings.append(f"{rel}: reserved file has frontmatter keys {sorted(fm)}")

    for md in iter_concept_paths(knowledge):
        rel = md.relative_to(root).as_posix()
        cid = concept_id(md, knowledge)
        fm, body, has_fm = load_frontmatter(md)
        if not has_fm or fm is None:
            errors.append(f"{rel}: missing parseable YAML frontmatter")
            continue
        if "type" not in fm:
            errors.append(f"{rel}: missing required OKF type")

        extra_keys = set(fm) - KNOWN_KEYS
        if extra_keys:
            warnings.append(f"{rel}: extra keys {sorted(extra_keys)} (allowed)")

        pass1 = fresh.get(cid, {})
        p1_fields = []
        det = pass1.get("deterministic")
        if isinstance(det, dict):
            p1_fields = list(det.get("fields") or [])

        disk_det = fm.get("deterministic")
        if p1_fields:
            if not isinstance(disk_det, dict):
                errors.append(f"{rel}: deterministic block absent")
            else:
                disk_fields = disk_det.get("fields")
                if not disk_fields:
                    errors.append(f"{rel}: deterministic.fields missing")
                elif _as_str_set(disk_fields) != set(p1_fields):
                    errors.append(
                        f"{rel}: deterministic.fields {disk_fields!r} != extract {p1_fields!r}"
                    )
                if disk_det.get("by") != DETERMINISTIC_BY:
                    errors.append(f"{rel}: deterministic.by {disk_det.get('by')!r} != {DETERMINISTIC_BY!r}")

        for key in COMPARE_FIELDS:
            if key not in pass1:
                continue
            if key not in fm:
                errors.append(f"{rel}: pass-1 field {key!r} missing on disk")
                continue
            if key == "title" and pass1.get("type") != "RFC" and cid != "crate":
                # Only RFC/crate titles are pass-1 facts.
                if key not in p1_fields:
                    continue
            if not values_equal(fm[key], pass1[key]):
                errors.append(f"{rel}: {key} {fm[key]!r} != extract {pass1[key]!r}")

        if "tags" in pass1:
            missing = _as_str_set(pass1["tags"]) - _as_str_set(fm.get("tags"))
            if missing:
                errors.append(f"{rel}: tags missing pass-1 values {sorted(missing)}")

        if "td_ids" in pass1:
            disk_ids = fm.get("td_ids")
            if _as_str_set(disk_ids) != _as_str_set(pass1["td_ids"]):
                errors.append(f"{rel}: td_ids {disk_ids!r} != extract {pass1['td_ids']!r}")

        if "sources" in pass1 and isinstance(pass1["sources"], list):
            disk_sources = fm.get("sources") if isinstance(fm.get("sources"), list) else []
            disk_by_id = {
                e.get("id"): e for e in disk_sources if isinstance(e, dict) and e.get("id") is not None
            }
            for entry in pass1["sources"]:
                if not isinstance(entry, dict) or "id" not in entry:
                    continue
                disk_e = disk_by_id.get(entry["id"])
                if disk_e is None:
                    errors.append(f"{rel}: sources missing pass-1 id {entry['id']!r}")
                elif disk_e.get("resource") != entry.get("resource"):
                    errors.append(
                        f"{rel}: sources id {entry['id']!r} resource {disk_e.get('resource')!r} "
                        f"!= extract {entry.get('resource')!r}"
                    )

        sources = fm.get("sources")
        if isinstance(sources, list):
            seen_id: dict[str, str] = {}
            for entry in sources:
                if not isinstance(entry, dict) or "id" not in entry:
                    continue
                sid = str(entry["id"])
                res = str(entry.get("resource", ""))
                if sid in seen_id and seen_id[sid] != res:
                    errors.append(f"{rel}: duplicate sources id {sid!r} with different resource")
                seen_id.setdefault(sid, res)

        humans = _human_actors(fm.get("verified"))
        if humans:
            allowed = main_human_verifiers(root, rel)
            invented = humans - allowed
            if invented:
                errors.append(f"{rel}: invented human verified actors {sorted(invented)}")

        rfc_m = RFC_STEM_RE.match(md.stem)
        if fm.get("type") == "RFC" or (cid.startswith("rfcs/") and rfc_m):
            txt = md.with_name(f"{md.stem}.txt")
            if not txt.is_file():
                errors.append(f"{rel}: RFC concept without sibling {txt.name}")
            elif rfc_m:
                header = parse_rfc_header(txt)
                file_num = int(rfc_m.group(1))
                hdr_num = header.get("header_rfc_number")
                if hdr_num is not None and hdr_num != file_num:
                    errors.append(
                        f"{rel}: filename RFC {file_num} != Request for Comments: {hdr_num}"
                    )

        if cid in PLUGTEST_YML:
            yml = md.parent / PLUGTEST_YML[cid]
            if yml.is_file():
                allowed_ids = set(parse_td_ids(yml))
                disk_ids = _as_str_set(fm.get("td_ids"))
                extra = disk_ids - allowed_ids
                if extra:
                    errors.append(f"{rel}: td_ids not in vendored YAML: {sorted(extra)}")

        if body:
            for match in re.finditer(r"\[[^\]]*\]\(([^)]+)\)", body):
                href = match.group(1).split("#", 1)[0]
                if not href or href.startswith(("http://", "https://", "mailto:")):
                    continue
                if not href.endswith(".md"):
                    continue
                if href.startswith("/"):
                    target = knowledge / href.lstrip("/")
                else:
                    target = (md.parent / href).resolve()
                try:
                    target.relative_to((knowledge).resolve())
                except ValueError:
                    if href.startswith("../") and (md.parent / href).exists():
                        continue
                    warnings.append(f"{rel}: cross-link {href} leaves knowledge/")
                    continue
                if not (md.parent / href).exists() and not target.exists():
                    warnings.append(f"{rel}: broken optional cross-link {href}")

    for msg in warnings:
        print(f"WARN {msg}", file=sys.stderr)
    for msg in errors:
        print(f"NO-GO {msg}", file=sys.stderr)
    if errors:
        print(f"--validate NO-GO: {len(errors)} critical, {len(warnings)} warning(s)", file=sys.stderr)
        return 1
    print(f"ok: validate GO ({len(warnings)} warning(s))")
    return 0


def self_test(root: Path) -> int:
    txt = root / "knowledge" / "rfcs" / "rfc7252.txt"
    header = parse_rfc_header(txt)
    errors = 0
    expected_title = "The Constrained Application Protocol (CoAP)"
    if header.get("title") != expected_title:
        print(f"self-test: title {header.get('title')!r} != {expected_title!r}", file=sys.stderr)
        errors += 1
    if header.get("header_rfc_number") != 7252:
        print(f"self-test: RFC number {header.get('header_rfc_number')!r}", file=sys.stderr)
        errors += 1
    if header.get("ietf_status") != "Standards Track":
        print(f"self-test: ietf_status {header.get('ietf_status')!r}", file=sys.stderr)
        errors += 1
    if header.get("date") != "June 2014":
        print(f"self-test: date {header.get('date')!r}", file=sys.stderr)
        errors += 1
    cargo = parse_cargo(root / "Cargo.toml")
    if cargo.get("crate_name") != "coaptic":
        print(f"self-test: crate_name {cargo.get('crate_name')!r}", file=sys.stderr)
        errors += 1
    ci = parse_workflow_triggers((root / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8"))
    if ci != ["on.pull_request", "workflow_dispatch"]:
        print(f"self-test: ci triggers {ci!r}", file=sys.stderr)
        errors += 1
    rel = parse_workflow_triggers((root / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8"))
    if rel != ["on.push.tags", "workflow_dispatch"]:
        print(f"self-test: release triggers {rel!r}", file=sys.stderr)
        errors += 1
    if errors:
        print(f"--self-test failed: {errors} error(s)", file=sys.stderr)
        return 1
    print("ok: self-test")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", "-o", help="Write the pass-1 YAML mapping to this path")
    parser.add_argument(
        "--check",
        action="store_true",
        help="Fail if an RFC concept title/resource disagrees with the sibling txt",
    )
    parser.add_argument(
        "--validate",
        action="store_true",
        help="Pass 3: review finished concept files on disk (NO-GO on critical)",
    )
    parser.add_argument("--self-test", action="store_true", help="Tiny parser assertions")
    parser.add_argument("--root", default=None, help="Repo root (default: discover from script/cwd)")
    args = parser.parse_args(argv)
    root = Path(args.root).resolve() if args.root else repo_root()

    if args.self_test:
        return self_test(root)
    if args.validate:
        return validate(root)
    if args.check:
        return check_rfc_concepts(root)

    mapping = extract_all(root)
    yaml_text = dump_mapping(mapping)
    if args.output:
        out = Path(args.output)
        out.write_text(yaml_text, encoding="utf-8")
        print(f"wrote {out} ({len(mapping)} concepts)", file=sys.stderr)
    else:
        sys.stdout.write(yaml_text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
