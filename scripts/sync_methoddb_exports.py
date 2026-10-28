#!/usr/bin/env python3
"""Ensure every MethodDB entry from git HEAD has #[deerpunk_engine_macro::export] on its tramp."""

import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
API = ROOT / "crates" / "host" / "engine_api"

RENAME = {
    "v1:um:register": "v1:net:msg_register",
    "v1:um:send": "v1:net:msg_send",
    "v1:um:hook": "v1:net:msg_hook",
}

FILE_SCOPE = {
    "world.rs": ["server"],
    "world_client.rs": ["client"],
    "imgui.rs": ["client"],
    "input.rs": ["client"],
    "renderer.rs": ["client"],
    "ui.rs": ["client"],
}

SUBMIT_RE = re.compile(
    r'name:\s*"(?P<name>[^"]+)"\s*,\s*callback:\s*(?P<cb>\w+)\s+as',
)


def scope_for(name: str, stem: str) -> str:
    if name.startswith("v1:server:"):
        scopes = ["server"]
    elif name.startswith("v1:client:"):
        scopes = ["client"]
    elif name.startswith("v1:move:"):
        scopes = ["client"]
    elif stem in FILE_SCOPE:
        scopes = FILE_SCOPE[stem]
    else:
        scopes = ["server", "client"]
    inner = ", ".join(f'"{s}"' for s in scopes)
    return f"scope = [{inner}]"


def git_entries(path: Path) -> list[tuple[str, str]]:
    rel = path.relative_to(ROOT).as_posix()
    try:
        raw = subprocess.check_output(
            ["git", "show", f"HEAD:{rel}"],
            cwd=ROOT,
            text=True,
            errors="replace",
        )
    except subprocess.CalledProcessError:
        return []
    out = []
    for m in SUBMIT_RE.finditer(raw):
        name = RENAME.get(m.group("name"), m.group("name"))
        out.append((name, m.group("cb")))
    return out


def export_attr(name: str, stem: str) -> str:
    return f'#[deerpunk_engine_macro::export(name = "{name}", {scope_for(name, stem)})]\n'


def has_export_above(text: str, pos: int) -> bool:
    for line in reversed(text[:pos].rstrip().split("\n")[-8:]):
        s = line.strip()
        if not s or s.startswith("//"):
            continue
        return "deerpunk_engine_macro::export" in s
    return False


def sync_file(path: Path) -> int:
    entries = git_entries(path)
    if not entries:
        return 0
    text = path.read_text(encoding="utf-8")
    stem = path.stem
    fixed = 0
    for api_name, cb in entries:
        pat = re.compile(
            r"(?m)^[ \t]*pub extern \"C\" fn " + re.escape(cb) + r"\s*[\(]"
        )
        m = pat.search(text)
        if not m:
            print(f"WARN {path.name}: missing fn {cb} for {api_name}")
            continue
        start = m.start()
        if has_export_above(text, start):
            continue
        text = text[:start] + export_attr(api_name, stem) + text[start:]
        fixed += 1
    if fixed:
        text = text.replace(
            "use crate::engine_api::engine_api::EngineApiMethod;\n\n",
            "",
        )
        text = text.replace(
            "use crate::engine_api::engine_api::EngineApiMethod;\n",
            "",
        )
        path.write_text(text, encoding="utf-8")
    return fixed


def main() -> None:
    total = 0
    for path in sorted(API.glob("*.rs")):
        if path.name in ("engine_api.rs", "callback_loader.rs", "mod.rs"):
            continue
        n = sync_file(path)
        if n:
            print(f"{path.name}: +{n} export(s)")
            total += n
    print(f"done, fixed {total}")


if __name__ == "__main__":
    main()
