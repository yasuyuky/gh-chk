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
    Prs { user: String },
    /// Contriburions
    Contributions { user: String },
    /// Notifications
    Notifications,
}

#[async_std::main]
async fn main() -> surf::Result<()> {
    let opt = Opt::from_args();
    match opt.command {
        Command::Prs { user } => cmd::prs::check(&user).await?,
        Command::Contributions { user } => cmd::contributions::check(&user).await?,
        Command::Notifications => cmd::notifications::check().await?,
    };
    Ok(())
}
