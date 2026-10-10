# RmlUi third-party layout

Used by `crates/ui` (`deerpunk-ui`).

| Path | Source |
|------|--------|
| `rmlui-sys/` | In-repo C API wrapper |
| `RmlUi/` | submodule `https://github.com/mikke89/RmlUi.git` |
| `freetype/` | submodule `https://github.com/freetype/freetype.git` |
| `lua-src/` | submodule `https://github.com/lua/lua.git` (branch `v5.4`) + `CMakeLists.txt` |

```bash
git submodule update --init --recursive
cargo run -p deerpunk-ui --example ui_hello
```
