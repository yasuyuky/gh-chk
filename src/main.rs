use once_cell::sync::Lazy;
use read_input::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use structopt::StructOpt;

mod cmd;
mod graphql;
mod rest;

#[derive(StructOpt)]
struct Opt {
    #[structopt(subcommand)]
    command: Command,
}
#[derive(Debug, StructOpt)]
#[structopt(rename_all = "kebab-case")]
enum Command {
    /// PRs
    Prs { owner: Option<String> },
    /// Contriburions
    Contributions { user: Option<String> },
    /// Notifications
    Notifications { page: usize },
    /// Login
    Login,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    token: Option<String>,
}

impl Config {
    pub fn new() -> Self {
        Self { token: None }
    }

    pub fn from_path(p: &Path) -> Self {
        let mut s = String::new();
        match File::open(p).and_then(|mut f| f.read_to_string(&mut s)) {
            Ok(_) => toml::from_str(&s).unwrap_or(Self::new()),
            Err(_) => Self::new(),
        }
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

fn login() -> Result<(), std::io::Error> {
    let token: String = input()
        .msg("Input your GitHub Personal Access Token: ")
        .get();
    let conf = Config { token: Some(token) };
    let s = toml::to_string(&conf).unwrap();
    let path = CONFIG_PATH.clone();
    let dir = path.parent().unwrap();
    if !dir.exists() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, s)
}

#[async_std::main]
async fn main() -> surf::Result<()> {
    let opt = Opt::from_args();
    match opt.command {
        Command::Prs { owner } => cmd::prs::check(owner).await?,
        Command::Contributions { user } => cmd::contributions::check(user).await?,
        Command::Notifications { page } => cmd::notifications::list(page).await?,
        Command::Login => login()?,
    };
    Ok(())
}
