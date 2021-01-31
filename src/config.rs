use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub token: Option<String>,
}

impl Config {
    pub fn new() -> Self {
        Self { token: None }
    }

    pub fn from_path(p: &Path) -> Self {
        let mut s = String::new();
        match File::open(p).and_then(|mut f| f.read_to_string(&mut s)) {
            Ok(_) => toml::from_str(&s).unwrap_or_default(),
            Err(_) => Self::new(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

pub static CONFIG_PATH: Lazy<PathBuf> = Lazy::new(|| {
    let mut path = match std::env::var("XDG_CONFIG_HOME") {
        Ok(p) => PathBuf::from(p),
        Err(_) => PathBuf::from(std::env::var("HOME").unwrap() + "/.config"),
    };
    path.push("ghchk");
    path.push("config.toml");
    path
});

pub static CONFIG: Lazy<Config> = Lazy::new(|| Config::from_path(&CONFIG_PATH));

pub static TOKEN: Lazy<String> = Lazy::new(|| match std::env::var("GITHUB_TOKEN") {
    Ok(tok) => tok,
    Err(_) => CONFIG.token.clone().unwrap_or_default(),
});
