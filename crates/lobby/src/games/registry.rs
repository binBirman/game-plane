use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameEntry {
    pub r#type: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub binary: PathBuf,
    #[serde(default = "default_min")]
    pub min_players: usize,
    #[serde(default = "default_max")]
    pub max_players: usize,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub variants: Vec<String>,
}

impl GameEntry {
    /// Resolve `binary` to a runnable file. Absolute / relative-with-dir
    /// paths are checked literally; bare names walk `PATH`.
    pub fn resolve_binary(&self) -> BinResolve {
        resolve_binary(&self.binary)
    }
}

/// Result of resolving a configured game binary.
#[derive(Debug)]
pub enum BinResolve {
    Ok,
    NotFound(String),
    NotExecutable,
}

fn default_min() -> usize {
    2
}
fn default_max() -> usize {
    2
}
fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GamesFile {
    #[serde(default)]
    pub games: Vec<GameEntry>,
}

pub struct GameRegistry {
    by_type: HashMap<String, GameEntry>,
    default_bin: PathBuf,
}

impl GameRegistry {
    pub fn new(default_bin: PathBuf) -> Self {
        Self {
            by_type: HashMap::new(),
            default_bin,
        }
    }

    pub fn from_file(path: &Path, default_bin: PathBuf) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read games.toml {}", path.display()))?;
        let parsed: GamesFile = toml::from_str(&raw)
            .with_context(|| format!("parse games.toml {}", path.display()))?;
        let mut reg = Self::new(default_bin);
        for g in parsed.games {
            if !g.enabled {
                continue;
            }
            if reg.by_type.contains_key(&g.r#type) {
                warn!(game_type = %g.r#type, "duplicate game_type in registry, keeping first");
                continue;
            }
            reg.by_type.insert(g.r#type.clone(), g);
        }
        Ok(reg)
    }

    /// Fallback: register the default binary as the only game (type=tictactoe).
    pub fn with_default(mut self, game_type: &str, display_name: &str) -> Self {
        let entry = GameEntry {
            r#type: game_type.into(),
            name: display_name.into(),
            description: String::new(),
            binary: self.default_bin.clone(),
            min_players: 2,
            max_players: 2,
            enabled: true,
            variants: vec![],
        };
        self.by_type.insert(entry.r#type.clone(), entry);
        self
    }

    pub fn get(&self, game_type: &str) -> Option<&GameEntry> {
        self.by_type.get(game_type)
    }

    pub fn list_enabled(&self) -> Vec<&GameEntry> {
        let mut v: Vec<&GameEntry> = self.by_type.values().collect();
        v.sort_by(|a, b| a.r#type.cmp(&b.r#type));
        v
    }
}

pub fn public_view(g: &GameEntry) -> serde_json::Value {
    serde_json::json!({
        "type": g.r#type,
        "name": g.name,
        "description": g.description,
        "min_players": g.min_players,
        "max_players": g.max_players,
        "variants": g.variants,
    })
}

/// Resolve a configured game binary path. Distinguishes "missing" from
/// "exists but not executable" so the start handler can return a useful
/// `GAME_BINARY_NOT_FOUND` instead of an opaque 500.
pub fn resolve_binary(bin: &Path) -> BinResolve {
    use std::os::unix::fs::PermissionsExt;

    // Absolute or relative-with-dir → check the literal path.
    if bin.is_absolute() || bin.components().count() > 1 {
        return match std::fs::metadata(bin) {
            Err(e) => BinResolve::NotFound(e.to_string()),
            Ok(md) if !md.is_file() => BinResolve::NotFound("not a regular file".into()),
            Ok(md) => {
                if md.permissions().mode() & 0o111 == 0 {
                    BinResolve::NotExecutable
                } else {
                    BinResolve::Ok
                }
            }
        };
    }

    // Bare name → walk PATH.
    let Some(paths) = std::env::var_os("PATH") else {
        return BinResolve::NotFound("PATH env var not set".into());
    };
    let mut tried = Vec::new();
    for p in std::env::split_paths(&paths) {
        let candidate = Path::new(&p).join(bin);
        tried.push(candidate.display().to_string());
        if let Ok(md) = std::fs::metadata(&candidate) {
            if md.is_file() {
                return if md.permissions().mode() & 0o111 != 0 {
                    BinResolve::Ok
                } else {
                    BinResolve::NotExecutable
                };
            }
        }
    }
    BinResolve::NotFound(format!("searched PATH: {}", tried.join(", ")))
}