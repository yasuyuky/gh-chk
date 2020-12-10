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
}

#[async_std::main]
async fn main() -> surf::Result<()> {
    let opt = Opt::from_args();
    match opt.command {
        Command::Prs { owner } => cmd::prs::check(owner).await?,
        Command::Contributions { user } => cmd::contributions::check(user).await?,
        Command::Notifications { page } => cmd::notifications::list(page).await?,
    };
    Ok(())
}
