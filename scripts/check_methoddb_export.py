#!/usr/bin/env python3
import re
from pathlib import Path

root = Path(__file__).resolve().parents[1] / "crates" / "host" / "engine_api"
for p in sorted(root.glob("*.rs")):
    t = p.read_text(encoding="utf-8")
    missing = []
    for m in re.finditer(r'(?m)^[ \t]*pub extern "C" fn (\w+)', t):
        fn = m.group(1)
        start = m.start()
        pre = t[max(0, start - 400) : start]
        if not re.search(r"#\[deerpunk_macro::export\([^\]]+\)\]\s*$", pre.rstrip()[-120:]):
            if "deerpunk_macro::export" not in pre.split("\n")[-3:]:
                missing.append(fn)
    if missing:
        print(f"{p.name}: {', '.join(missing)}")
