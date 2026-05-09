use crate::env_keys::{
    ENV_GH_CHK_API_BASE_URL, ENV_GH_CHK_GRAPHQL_URL, ENV_GITHUB_TOKEN, ENV_HOME,
    ENV_XDG_CONFIG_HOME,
};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Config {
    pub token: Option<String>,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum Format {
    Text,
    Json,
}

impl Config {
    pub fn new() -> Self {
        Self { token: None }
    }

    pub fn from_path(p: &Path) -> Self {
        let mut s = String::default();
        match File::open(p).and_then(|mut f| f.read_to_string(&mut s)) {
            Ok(_) => toml::from_str(&s).unwrap_or_default(),
            Err(_) => Self::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TokenEntry {
    user: Option<String>,
    oauth_token: Option<String>,
    git_protocol: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct GHConfig {
    #[serde(flatten)]
    entries: HashMap<String, TokenEntry>,
}

impl GHConfig {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn from_path(p: &Path) -> Self {
        let mut s = String::default();
        match File::open(p).and_then(|mut f| f.read_to_string(&mut s)) {
            Ok(_) => serde_yaml::from_str(&s).unwrap_or_default(),
            Err(_) => Self::new(),
        }
    }

    fn token_for(&self, host: &str) -> Option<String> {
        self.entries
            .get(host)
            .and_then(|entry| clean_token(entry.oauth_token.clone()))
    }
}

pub static CONFIG_PATH: Lazy<PathBuf> = Lazy::new(|| {
    let mut path = std::env::var(ENV_XDG_CONFIG_HOME)
        .map(PathBuf::from)
        .or_else(|_| std::env::var(ENV_HOME).map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|_| PathBuf::from(".config"));
    path.push("gh-chk");
    path.push("config.toml");
    path
});

pub static CONFIG: Lazy<Config> = Lazy::new(|| Config::from_path(&CONFIG_PATH));

pub static GH_CONFIG_PATH: Lazy<PathBuf> = Lazy::new(|| {
    let mut path = std::env::var(ENV_XDG_CONFIG_HOME)
        .map(PathBuf::from)
        .or_else(|_| std::env::var(ENV_HOME).map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|_| PathBuf::from(".config"));
    path.push("gh");
    path.push("hosts.yml");
    path
});

pub static GH_CONFIG: Lazy<GHConfig> = Lazy::new(|| GHConfig::from_path(&GH_CONFIG_PATH));

pub static TOKEN: Lazy<String> = Lazy::new(resolve_token);

pub static FORMAT: OnceLock<Format> = OnceLock::new();

fn clean_token(token: Option<String>) -> Option<String> {
    token
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
}

fn gh_auth_token(host: &str) -> Option<String> {
    let output = Command::new("gh")
        .args(["auth", "token", "-h", host])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    clean_token(Some(String::from_utf8_lossy(&output.stdout).to_string()))
}

fn resolve_token() -> String {
    GH_CONFIG
        .token_for("github.com")
        .or_else(|| gh_auth_token("github.com"))
        .or_else(|| clean_token(CONFIG.token.clone()))
        .or_else(|| clean_token(std::env::var(ENV_GITHUB_TOKEN).ok()))
        .unwrap_or_default()
}

fn normalized_env_url(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|url| url.trim_end_matches('/').to_owned())
        .filter(|url| !url.is_empty())
}

pub fn github_api_base_url() -> String {
    normalized_env_url(ENV_GH_CHK_API_BASE_URL).unwrap_or_else(|| "https://api.github.com".into())
}

pub fn github_api_url(path: &str) -> String {
    format!("{}/{}", github_api_base_url(), path.trim_start_matches('/'))
}

pub fn github_graphql_url() -> String {
    normalized_env_url(ENV_GH_CHK_GRAPHQL_URL)
        .unwrap_or_else(|| format!("{}/graphql", github_api_base_url()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_paths_resolve_without_home() {
        let orig_home = std::env::var_os(ENV_HOME);
        let orig_xdg = std::env::var_os(ENV_XDG_CONFIG_HOME);
        unsafe {
            std::env::remove_var(ENV_HOME);
            std::env::remove_var(ENV_XDG_CONFIG_HOME);
        }

        let conf = CONFIG_PATH.clone();
        let gh_conf = GH_CONFIG_PATH.clone();

        assert!(conf.ends_with("gh-chk/config.toml"));
        assert!(gh_conf.ends_with("gh/hosts.yml"));

        unsafe {
            if let Some(val) = orig_home {
                std::env::set_var(ENV_HOME, val);
            }
            if let Some(val) = orig_xdg {
                std::env::set_var(ENV_XDG_CONFIG_HOME, val);
            }
        }
    }

    #[test]
    fn github_urls_can_be_overridden() {
        let orig_api = std::env::var_os(ENV_GH_CHK_API_BASE_URL);
        let orig_graphql = std::env::var_os(ENV_GH_CHK_GRAPHQL_URL);
        unsafe {
            std::env::set_var(ENV_GH_CHK_API_BASE_URL, "http://localhost:8080/");
            std::env::set_var(ENV_GH_CHK_GRAPHQL_URL, "http://localhost:8081/graphql/");
        }

        assert_eq!(github_api_base_url(), "http://localhost:8080");
        assert_eq!(
            github_api_url("search/code"),
            "http://localhost:8080/search/code"
        );
        assert_eq!(github_graphql_url(), "http://localhost:8081/graphql");

        unsafe {
            match orig_api {
                Some(val) => std::env::set_var(ENV_GH_CHK_API_BASE_URL, val),
                None => std::env::remove_var(ENV_GH_CHK_API_BASE_URL),
            }
            match orig_graphql {
                Some(val) => std::env::set_var(ENV_GH_CHK_GRAPHQL_URL, val),
                None => std::env::remove_var(ENV_GH_CHK_GRAPHQL_URL),
            }
        }
    }

    #[test]
    fn gh_config_reads_plaintext_token() {
        let conf: GHConfig = serde_yaml::from_str(
            r#"
github.com:
  user: octocat
  oauth_token: test-token
  git_protocol: https
"#,
        )
        .unwrap();

        assert_eq!(conf.token_for("github.com"), Some("test-token".to_string()));
    }

    #[test]
    fn gh_config_allows_keyring_only_hosts_file() {
        let conf: GHConfig = serde_yaml::from_str(
            r#"
github.com:
  user: octocat
  git_protocol: https
  users:
    octocat: {}
"#,
        )
        .unwrap();

        assert_eq!(conf.token_for("github.com"), None);
    }
}
