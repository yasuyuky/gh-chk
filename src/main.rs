use clap::Parser;
use config::Format;
use read_input::prelude::*;

mod cmd;
mod config;
mod graphql;
mod rest;

#[derive(Parser)]
struct Opt {
    #[clap(subcommand)]
    command: Command,
    #[clap(short = 'f', default_value = "text")]
    format: Format,
}

#[derive(Debug, Parser)]
#[clap(rename_all = "kebab-case")]
enum Command {
    /// Show pullrequests of the repository or user
    Prs { slug: Vec<String> },
    /// Show issues of the repository or user
    Issues { slug: Vec<String> },
    /// Show contriburions of the user
    #[clap(alias = "grass")]
    Contributions { user: Option<String> },
    /// Show notifications of the user
    Notifications { page: usize },
    /// Track assignees of the issues or pullrequests
    TrackAssignees { slug: String, num: usize },
    /// Search repositories
    Search(cmd::search::Query),
    /// Login to GitHub
    Login,
    /// Logout to GitHub
    Logout,
}

fn login() -> Result<(), std::io::Error> {
    let token: String = input()
        .msg("Input your GitHub Personal Access Token: ")
        .get();
    let conf = config::Config { token: Some(token) };
    let s = toml::to_string(&conf).unwrap();
    let path = config::CONFIG_PATH.clone();
    let dir = path.parent().unwrap();
    if !dir.exists() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, s)
}

fn logout() -> Result<(), std::io::Error> {
    let path = config::CONFIG_PATH.clone();
    if path.exists() {
        std::fs::remove_file(&path)
    } else {
        Ok(())
    }
}

#[async_std::main]
async fn main() -> surf::Result<()> {
    let opt = Opt::parse();
    config::FORMAT.set(opt.format).expect("set format");
    match opt.command {
        Command::Prs { slug } => cmd::prs::check(slug).await?,
        Command::Issues { slug } => cmd::issues::check(slug).await?,
        Command::Contributions { user } => cmd::contributions::check(user).await?,
        Command::Notifications { page } => cmd::notifications::list(page).await?,
        Command::TrackAssignees { slug, num } => cmd::trackassignees::track(&slug, num).await?,
        Command::Search { query } => cmd::search::search(&query).await?,
        Command::Login => login()?,
        Command::Logout => logout()?,
    };
    Ok(())
}
