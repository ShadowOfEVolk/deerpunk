//! Persist / load host configs under `{gamedir}/configs/` (Xash-style split).
//!
//! - `video.cfg`  — cvars with [`crate::cvar::RENDERINFO`]
//! - `config.cfg` — keybinds + cvars with [`crate::cvar::ARCHIVE`]
//! - `autoexec.cfg` — user script (never overwritten by the engine)

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::binds::{self, key_name};
use crate::cmd::CmdSystem;
use crate::cvar::{self, CvarSystem};
use crate::fs::FileSystem;

const VIDEO_CFG: &str = "configs/video.cfg";
const CONFIG_CFG: &str = "configs/config.cfg";
const AUTOEXEC_CFG: &str = "configs/autoexec.cfg";

/// Exec archived configs then autoexec (host owns the order — like valve.rc fallback).
pub fn exec_startup(cmd: &CmdSystem, fs: &FileSystem) {
    exec_if_exists(cmd, fs, VIDEO_CFG);
    exec_if_exists(cmd, fs, CONFIG_CFG);
    exec_if_exists(cmd, fs, AUTOEXEC_CFG);
}

/// Write `video.cfg` + `config.cfg` (atomic `.new` → rename).
pub fn write_all(cvar: &CvarSystem, fs: &FileSystem) {
    if let Err(e) = write_video_cfg(cvar, fs) {
        log::warn!("[cfg] write {VIDEO_CFG}: {e}");
    }
    if let Err(e) = write_config_cfg(cvar, fs) {
        log::warn!("[cfg] write {CONFIG_CFG}: {e}");
    }
}

fn exec_if_exists(cmd: &CmdSystem, fs: &FileSystem, rel: &str) {
    let path = fs.resolve_path(rel);
    if !path.is_file() {
        return;
    }
    log::info!("[cfg] exec {}", path.display());
    cmd.exec_script_file(&path.to_string_lossy());
}

fn write_video_cfg(cvar: &CvarSystem, fs: &FileSystem) -> std::io::Result<()> {
    let path = fs.resolve_path(VIDEO_CFG);
    ensure_parent(&path)?;
    let mut body = String::new();
    body.push_str("// video.cfg — renderer archive (RENDERINFO). Auto-written.\n");
    for (name, value) in cvar.dump_flagged(cvar::RENDERINFO) {
        push_cvar_line(&mut body, &name, &value);
    }
    atomic_write(&path, &body)
}

fn write_config_cfg(cvar: &CvarSystem, fs: &FileSystem) -> std::io::Result<()> {
    let path = fs.resolve_path(CONFIG_CFG);
    ensure_parent(&path)?;
    let mut body = String::new();
    body.push_str("// config.cfg — binds + ARCHIVE cvars. Auto-written.\n");
    body.push_str("unbindall\n");
    for (key, command) in binds::global().list() {
        body.push_str(&format!(
            "bind \"{}\" \"{}\"\n",
            escape_cfg(key_name(key).as_str()),
            escape_cfg(&command)
        ));
    }
    body.push('\n');
    for (name, value) in cvar.dump_flagged(cvar::ARCHIVE) {
        push_cvar_line(&mut body, &name, &value);
    }
    atomic_write(&path, &body)
}

fn push_cvar_line(out: &mut String, name: &str, value: &str) {
    if value.is_empty() || value.contains(char::is_whitespace) {
        out.push_str(&format!("{name} \"{}\"\n", escape_cfg(value)));
    } else {
        out.push_str(&format!("{name} {value}\n"));
    }
}

fn escape_cfg(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn ensure_parent(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn atomic_write(path: &Path, body: &str) -> std::io::Result<()> {
    let tmp = PathBuf::from(format!("{}.new", path.display()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.flush()?;
    }
    // Windows: rename over existing may fail — remove first.
    let _ = fs::remove_file(path);
    fs::rename(&tmp, path)?;
    log::info!("[cfg] wrote {}", path.display());
    Ok(())
}
