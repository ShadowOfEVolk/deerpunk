use std::path::{Path, PathBuf};

use log::LevelFilter;

/// Paths passed on the command line; every flag takes one value.
struct LaunchArgs {
    gamedir: Option<PathBuf>,
    svlib: Option<PathBuf>,
    cllib: Option<PathBuf>,
    /// `--inspector-port <u16>`: forces `sv_inspector_port` (0 = disabled).
    inspector_port: Option<u16>,
    /// `--inspector-host <addr>`: forces `sv_inspector_host`.
    inspector_host: Option<String>,
}

/// Hand-rolled parser for `--gamedir`, `--svlib`, `--cllib`, `--inspector-port`
/// and `--inspector-host`.
///
/// Unknown flags and flags missing a following value are collected instead of
/// rejected so they can be reported once logging is initialized (parsing runs
/// before that).
///
/// Quake-style `+cmd args ...` sequences are skipped silently: the host
/// captures the raw argv itself (`CmdSystem::set_cmdline`) and replays them
/// via the `stuffcmds` console command after config exec.
fn parse_args(argv: &[String]) -> (LaunchArgs, Vec<String>) {
    let mut launch = LaunchArgs {
        gamedir: None,
        svlib: None,
        cllib: None,
        inspector_port: None,
        inspector_host: None,
    };
    let mut problems: Vec<String> = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        i += 1;
        if arg.starts_with('+') {
            // Consume the command's arguments; the next `+cmd` or `--flag`
            // starts a new sequence (same boundaries the host uses).
            while argv
                .get(i)
                .is_some_and(|a| !a.starts_with('+') && !a.starts_with("--"))
            {
                i += 1;
            }
            continue;
        }
        if !arg.starts_with("--") || arg.len() <= 2 {
            // Non-flag token is not part of any known invocation shape.
            problems.push(format!("unexpected argument: {}", arg));
            continue;
        }
        let name = &arg[2..];
        // Inspector flags carry string values (not paths); port validated here.
        if name == "inspector-port" || name == "inspector-host" {
            match argv.get(i) {
                Some(v) => {
                    i += 1;
                    if name == "inspector-host" {
                        launch.inspector_host = Some(v.clone());
                    } else {
                        match v.parse::<u16>() {
                            Ok(port) => launch.inspector_port = Some(port),
                            Err(_) => problems.push(format!(
                                "invalid value for --inspector-port: {v} (expected 0-65535)"
                            )),
                        }
                    }
                }
                None => problems.push(format!("--{} requires a value", name)),
            }
            continue;
        }
        // Borrow only the field matching the flag; arms are disjoint fields.
        let flag = match name {
            "gamedir" => &mut launch.gamedir,
            "svlib" => &mut launch.svlib,
            "cllib" => &mut launch.cllib,
            _ => {
                problems.push(format!("unknown flag: --{}", name));
                continue;
            }
        };
        // Last occurrence wins if a flag repeats.
        match argv.get(i) {
            Some(v) => {
                i += 1;
                *flag = Some(PathBuf::from(v));
            }
            None => problems.push(format!("--{} requires a value", name)),
        }
    }
    (launch, problems)
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let (cli, arg_problems) = parse_args(&argv);
    let engine_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let gamedir = resolve_gamedir(cli.gamedir.as_deref(), &cwd, &engine_root);

    // Run from the game's own directory (Xash-style): engine-relative paths
    // such as `maps/` must resolve inside the game folder, not the engine repo.
    let gamedir = match std::path::absolute(&gamedir) {
        Ok(p) => p,
        Err(_) => gamedir,
    };
    if let Err(e) = std::env::set_current_dir(&gamedir) {
        eprintln!("[deerpunk] chdir {} failed: {e}", gamedir.display());
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let log_dir = gamedir.join("logs");
    deerpunk_engine_common::logger::init(LevelFilter::Info, &log_dir)
        .expect("failed to init logger");

    for problem in &arg_problems {
        log::warn!("{}", problem);
    }

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

    // CLI inspector flags queue overrides for the `sv_inspector_*` cvars; they
    // are forced in when the inspector WS starts (beating config.cfg values).
    if let Some(port) = cli.inspector_port {
        host.cvar.queue_override("sv_inspector_port", &port.to_string());
        log::info!("CLI override: sv_inspector_port={port}");
    }
    if let Some(addr) = &cli.inspector_host {
        host.cvar.queue_override("sv_inspector_host", addr);
        log::info!("CLI override: sv_inspector_host={addr}");
    }

    let client_dll = resolve_client_dll(cli.cllib.as_deref(), &cwd, &gamedir, &engine_root);
    log::info!("CLIENT_DLL={}", client_dll.display());
    host.load_client_dll(&client_dll.to_string_lossy());

    let server_dll = resolve_server_dll(cli.svlib.as_deref(), &cwd, &gamedir, &engine_root);
    log::info!("SERVER_DLL={}", server_dll.display());
    host.load_server_dll(&server_dll.to_string_lossy());

    host.run();
}

/// Resolution priority: CLI `--gamedir` > env `GAMEDIR` > cwd heuristics
/// (configs/dlls dirs) > example gamedir > cwd.
fn resolve_gamedir(override_dir: Option<&Path>, cwd: &Path, engine_root: &Path) -> PathBuf {
    if let Some(p) = override_dir {
        return p.to_path_buf();
    }
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

/// Resolution priority: CLI `--svlib` > env `SERVER_DLL` > candidate list.
fn resolve_server_dll(
    override_path: Option<&Path>,
    cwd: &Path,
    gamedir: &Path,
    engine_root: &Path,
) -> PathBuf {
    if let Some(p) = override_path {
        return p.to_path_buf();
    }
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

/// Resolution priority: CLI `--cllib` > env `CLIENT_DLL` > candidate list.
fn resolve_client_dll(
    override_path: Option<&Path>,
    cwd: &Path,
    gamedir: &Path,
    engine_root: &Path,
) -> PathBuf {
    if let Some(p) = override_path {
        return p.to_path_buf();
    }
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
