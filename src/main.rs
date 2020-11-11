use serde::de::DeserializeOwned;
use structopt::StructOpt;

mod contributions;
mod prs;

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

async fn query<T: DeserializeOwned>(q: &serde_json::Value) -> surf::Result<T> {
    let uri = "https://api.github.com/graphql";
    let token = std::env::var("GITHUB_TOKEN")?;
    let mut res = surf::post(&uri)
        .header("Authorization", format!("bearer {}", token))
        .body(q.to_string())
        .await?;
    Ok(res.body_json::<T>().await?)
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
