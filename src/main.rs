use structopt::StructOpt;

mod contributions;
mod graphql;
mod prs;
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
}

#[async_std::main]
async fn main() -> surf::Result<()> {
    let opt = Opt::from_args();
    match opt.command {
        Command::Prs { user } => prs::check(&user).await?,
        Command::Contributions { user } => contributions::check(&user).await?,
    };
    Ok(())
}
