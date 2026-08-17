use std::path::{Path, PathBuf};

/// How to launch the engine binary.
#[derive(Debug, Clone)]
pub enum EngineLaunch {
    /// Direct binary path (e.g. "target/debug/tanks").
    Binary(PathBuf),
    /// Cargo run with features (e.g. `cargo run --features window --bin tanks`).
    Cargo {
        features: Option<String>,
        bin: String,
    },
}

/// Parsed project configuration from deerpunk.toml.
#[derive(Debug, Clone)]
pub struct ProjectConfig {
    /// Absolute path to the project directory (where deerpunk.toml lives).
    pub root: PathBuf,
    /// Project name.
    pub name: String,
    /// How to launch the engine.
    pub engine: EngineLaunch,
    /// Game directory (relative to project root, passed to engine).
    pub gamedir: String,
    /// Commands to send to engine on launch (without + prefix).
    pub commands: Vec<String>,
}

impl ProjectConfig {
    /// Load project config from a directory containing deerpunk.toml.
    pub fn load(project_dir: &Path) -> Result<Self, String> {
        let toml_path = project_dir.join("deerpunk.toml");
        let content = std::fs::read_to_string(&toml_path)
            .map_err(|e| format!("Cannot read {}: {e}", toml_path.display()))?;

        Self::parse(project_dir, &content)
    }

    /// Parse project config from TOML string.
    fn parse(project_dir: &Path, content: &str) -> Result<Self, String> {
        let table: toml::Table = toml::from_str(content)
            .map_err(|e| format!("Invalid deerpunk.toml: {e}"))?;

        let name = table
            .get("project")
            .and_then(|p| p.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("deerpunk")
            .to_string();

        // Engine launch: either [engine.cargo] or [engine] bin
        let engine = if let Some(cargo_section) = table
            .get("engine")
            .and_then(|e| e.get("cargo"))
        {
            let bin = cargo_section
                .get("bin")
                .and_then(|v| v.as_str())
                .unwrap_or("deerpunk")
                .to_string();
            let features = cargo_section
                .get("features")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            EngineLaunch::Cargo { features, bin }
        } else {
            let bin_path = table
                .get("engine")
                .and_then(|e| e.get("bin"))
                .and_then(|v| v.as_str())
                .unwrap_or("target/debug/deerpunk");
            EngineLaunch::Binary(project_dir.join(bin_path))
        };

        let gamedir = table
            .get("engine")
            .and_then(|e| e.get("gamedir"))
            .and_then(|v| v.as_str())
            .unwrap_or("examples/deathmatch/gamedir")
            .to_string();

        let commands = table
            .get("launch")
            .and_then(|l| l.get("commands"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        Ok(Self {
            root: project_dir.to_path_buf(),
            name,
            engine,
            gamedir,
            commands,
        })
    }
}
