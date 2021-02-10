use read_input::prelude::*;
use structopt::StructOpt;

mod cmd;
mod config;
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
    Prs { slug: Option<String> },
    /// Contriburions
    Contributions { user: Option<String> },
    /// Notifications
    Notifications { page: usize },
    /// Login
    Login,
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

#[async_std::main]
async fn main() -> surf::Result<()> {
    let opt = Opt::from_args();
    match opt.command {
        Command::Prs { slug } => cmd::prs::check(slug).await?,
        Command::Contributions { user } => cmd::contributions::check(user).await?,
        Command::Notifications { page } => cmd::notifications::list(page).await?,
        Command::Login => login()?,
    };
    Ok(())
}
