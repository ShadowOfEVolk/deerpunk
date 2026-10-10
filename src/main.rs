use std::path::{Path, PathBuf};

use log::LevelFilter;

fn main() {
    let engine_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let gamedir = resolve_gamedir(&cwd, &engine_root);

    let log_dir = gamedir.join("logs");
    deerpunk_engine_common::logger::init(LevelFilter::Info, &log_dir)
        .expect("failed to init logger");

    log::info!(
        "main() starting, cwd={}, engine_root={}, gamedir={}, log={}",
        cwd.display(),
        engine_root.display(),
        gamedir.display(),
        log_dir.join("latest.log").display()
    );

    // Shaders live with the game (`{gamedir}/shaders`), not in the engine repo.
    if std::env::var_os("SHADER_DIR").is_none() {
        let shaders = gamedir.join("shaders");
        if shaders.is_dir() {
            std::env::set_var("SHADER_DIR", &shaders);
            log::info!("SHADER_DIR={}", shaders.display());
        } else {
            log::warn!("no shaders dir at {}", shaders.display());
        }
    }

    let mut host = deerpunk_engine_host::Host::new();
    host.initialize(&gamedir);

    let client_dll = resolve_client_dll(&cwd, &gamedir, &engine_root);
    log::info!("CLIENT_DLL={}", client_dll.display());
    host.load_client_dll(&client_dll.to_string_lossy());

    let server_dll = resolve_server_dll(&cwd, &gamedir, &engine_root);
    log::info!("SERVER_DLL={}", server_dll.display());
    host.load_server_dll(&server_dll.to_string_lossy());

    host.run();
}

fn resolve_gamedir(cwd: &Path, engine_root: &Path) -> PathBuf {
    if let Ok(g) = std::env::var("GAMEDIR") {
        return PathBuf::from(g);
    }
    // Launch from game repo (tanks/): cwd itself is the gamedir.
    if cwd.join("configs").is_dir() || cwd.join("dlls").is_dir() {
        return cwd.to_path_buf();
    }
    // Launch from deerpunk repo: example gamedir.
    let example = engine_root.join("examples/deathmatch/gamedir");
    if example.is_dir() {
        return example;
    }
    cwd.to_path_buf()
}

fn resolve_server_dll(cwd: &Path, gamedir: &Path, engine_root: &Path) -> PathBuf {
    if let Ok(p) = std::env::var("SERVER_DLL") {
        return PathBuf::from(p);
    }
    let candidates = [
        gamedir.join("dlls/server/darkdemo_amd64.dll"),
        cwd.join("dlls/server/darkdemo_amd64.dll"),
        cwd.join("dlls/server.dll"),
        gamedir.join("dlls/server.dll"),
        engine_root.join("dlls/server.dll"),
    ];
    for c in &candidates {
        if c.is_file() {
            return c.clone();
        }
    }
    candidates[0].clone()
}

fn resolve_client_dll(cwd: &Path, gamedir: &Path, engine_root: &Path) -> PathBuf {
    if let Ok(p) = std::env::var("CLIENT_DLL") {
        return PathBuf::from(p);
    }
    let candidates = [
        gamedir.join("dlls/client/client_amd64.dll"),
        cwd.join("dlls/client/client_amd64.dll"),
        gamedir.join("dlls/client.dll"),
        cwd.join("dlls/client.dll"),
        engine_root.join("dlls/client.dll"),
    ];
    for c in &candidates {
        if c.is_file() {
            return c.clone();
        }
    }
    candidates[0].clone()
}
