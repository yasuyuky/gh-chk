use clap::Parser;
use config::Format;
use read_input::prelude::*;

mod cmd;
mod config;
mod env_keys;
mod graphql;
mod rest;
mod slug;
mod styling;

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
    Prs {
        slug: Vec<String>,
        #[clap(long)]
        merge: bool,
    },
    /// Interactive TUI for pull requests
    Tui {
        #[clap(
            long,
            value_name = "SECONDS",
            default_value_t = cmd::tui::AUTO_RELOAD_DEFAULT_SECS,
            value_parser = clap::value_parser!(u64).range(cmd::tui::AUTO_RELOAD_MIN_SECS..)
        )]
        auto_reload: u64,
        slug: Vec<String>,
    },
    /// Show issues of the repository or user
    Issues { slug: Vec<String> },
    /// Show contriburions of the user
    #[clap(alias = "grass")]
    Contributions { user: Option<String> },
    /// Show notifications of the user
    Notifications {
        #[clap(long = "read")]
        read: bool,
    },
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
    let s = toml::to_string(&conf).map_err(std::io::Error::other)?;
    let path = config::CONFIG_PATH.clone();
    let dir = match path.parent() {
        Some(d) => d,
        None => {
            return Err(std::io::Error::other(
                "invalid config path: no parent directory",
            ));
        }
    };
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
        Command::Prs { slug, merge } => cmd::prs::check(slug, merge).await?,
        Command::Tui { slug, auto_reload } => cmd::tui::run(slug, auto_reload).await?,
        Command::Issues { slug } => cmd::issues::check(slug).await?,
        Command::Contributions { user } => cmd::contributions::check(user).await?,
        Command::Notifications { read } => cmd::notifications::list(read).await?,
        Command::TrackAssignees { slug, num } => cmd::trackassignees::track(&slug, num).await?,
        Command::Search(q) => cmd::search::search(&q).await?,
        Command::Login => login()?,
        Command::Logout => logout()?,
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tui_auto_reload_defaults_to_enabled() {
        let opt = Opt::try_parse_from(["gh-chk", "tui", "foo"]).expect("parse args");
        match opt.command {
            Command::Tui { slug, auto_reload } => {
                assert_eq!(slug, vec!["foo".to_string()]);
                assert_eq!(auto_reload, Some(300));
            }
            _ => panic!("expected tui command"),
        }
    }

    #[test]
    fn tui_auto_reload_flag_uses_default_interval() {
        let opt =
            Opt::try_parse_from(["gh-chk", "tui", "--auto-reload", "foo"]).expect("parse args");
        match opt.command {
            Command::Tui { slug, auto_reload } => {
                assert_eq!(slug, vec!["foo".to_string()]);
                assert_eq!(auto_reload, Some(300));
            }
            _ => panic!("expected tui command"),
        }
    }

    #[test]
    fn tui_auto_reload_accepts_explicit_interval() {
        let opt =
            Opt::try_parse_from(["gh-chk", "tui", "--auto-reload=60", "foo"]).expect("parse args");
        match opt.command {
            Command::Tui { slug, auto_reload } => {
                assert_eq!(slug, vec!["foo".to_string()]);
                assert_eq!(auto_reload, Some(60));
            }
            _ => panic!("expected tui command"),
        }
    }

    #[test]
    fn tui_auto_reload_rejects_short_interval() {
        assert!(Opt::try_parse_from(["gh-chk", "tui", "--auto-reload=59"]).is_err());
    }
}
