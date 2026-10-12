#!/usr/bin/env python3
"""inventory::submit! -> #[deerpunk_macro::export(...)] on tramp fns."""

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "crates" / "host" / "engine_api"

FILE_SCOPE = {
    "world.rs": ["server"],
    "world_client.rs": ["client"],
    "imgui.rs": ["client"],
    "input.rs": ["client"],
    "renderer.rs": ["client"],
    "ui.rs": ["client"],
}

RENAME = {
    "v1:um:register": "v1:net:msg_register",
    "v1:um:send": "v1:net:msg_send",
    "v1:um:hook": "v1:net:msg_hook",
}

SUBMIT_RE = re.compile(
    r"inventory::submit!\s*\{\s*EngineApiMethod\s*\{[^}]*\}\s*\}\s*",
    re.MULTILINE,
)


def scope_for(name: str, file_stem: str) -> list[str]:
    if name.startswith("v1:server:"):
        return ["server"]
    if name.startswith("v1:client:"):
        return ["client"]
    if name.startswith("v1:move:"):
        return ["client"]
    if file_stem in FILE_SCOPE:
        return FILE_SCOPE[file_stem]
    return ["server", "client"]


def scope_lit(scopes: list[str]) -> str:
    return "scope = [" + ", ".join(f'"{s}"' for s in scopes) + "]"


def parse_submit_block(block: str) -> tuple[str, str]:
    m_name = re.search(r'name:\s*"([^"]+)"', block)
    m_cb = re.search(r"callback:\s*(\w+)\s+as\s+\*const", block)
    if not m_name or not m_cb:
        raise ValueError(f"cannot parse submit block:\n{block[:200]}")
    return m_name.group(1), m_cb.group(1)


def migrate_file(path: Path) -> bool:
    text = path.read_text(encoding="utf-8")
    matches = list(SUBMIT_RE.finditer(text))
    if not matches:
        return False

    stem = path.stem
    entries: list[tuple[str, str, list[str]]] = []
    for m in matches:
        raw_name, cb = parse_submit_block(m.group(0))
        api_name = RENAME.get(raw_name, raw_name)
        entries.append((cb, api_name, scope_for(api_name, stem)))

    text = SUBMIT_RE.sub("", text)

    def has_export_above(pos: int) -> bool:
        for line in reversed(text[:pos].rstrip().split("\n")[-8:]):
            s = line.strip()
            if not s or s.startswith("//"):
                continue
            return "deerpunk_macro::export" in s
        return False

    for cb, api_name, scopes in entries:
        attr = f'#[deerpunk_macro::export(name = "{api_name}", {scope_lit(scopes)})]\n'
        fn_pat = re.compile(
            rf'(?m)^[ \t]*pub extern "C" fn {re.escape(cb)}\s*\(',
        )
        m = fn_pat.search(text)
        if not m:
            raise SystemExit(f"{path.name}: fn {cb} not found for {api_name}")
        start = m.start()
        if not has_export_above(start):
            text = text[:start] + attr + text[start:]

    text = text.replace(
        "use crate::engine_api::engine_api::EngineApiMethod;\n\n",
        "",
    )
    text = text.replace(
        "use crate::engine_api::engine_api::EngineApiMethod;\n",
        "",
    )

    path.write_text(text, encoding="utf-8")
    print(f"migrated {path.name} ({len(entries)} methods)")
    return True


def main() -> None:
    for path in sorted(ROOT.glob("*.rs")):
        if path.name in ("engine_api.rs", "callback_loader.rs", "mod.rs"):
            continue
        migrate_file(path)


if __name__ == "__main__":
    main()
