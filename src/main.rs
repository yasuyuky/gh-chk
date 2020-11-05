use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;
use structopt::StructOpt;

#[derive(StructOpt)]
struct Opt {
    user: String,
}
#[derive(Deserialize)]
struct Res {
    data: Data,
}
#[derive(Deserialize)]
struct Data {
    user: User,
}
#[derive(Deserialize)]
struct User {
    repositories: RepositoryConnection,
}
#[derive(Deserialize)]
struct RepositoryConnection {
    nodes: Vec<Repository>,
}
#[allow(non_snake_case)]
#[derive(Deserialize)]
struct Repository {
    name: String,
    pullRequests: PullRequestsConnection,
}
#[derive(Deserialize)]
struct PullRequestsConnection {
    nodes: Vec<PullRequest>,
}
#[derive(Deserialize)]
struct PullRequest {
    number: usize,
    title: String,
    url: String,
}

async fn query<T: DeserializeOwned>(q: &str) -> surf::Result<T> {
    let uri = "https://api.github.com/graphql";
    let token = std::env::var("GITHUB_TOKEN")?;
    let mut res = surf::post(&uri)
        .header("Authorization", format!("bearer {}", token))
        .body(q)
        .await?;
    Ok(res.body_json::<T>().await?)
}

fn build_q(qstr: &str, user: &str) -> String {
    json!({
        "query": qstr,
        "variables": {"login": user}
    })
    .to_string()
}

async fn check_prs(user: &str) -> surf::Result<()> {
    let q = build_q(include_str!("query.user.repo.pr.graphql"), user);
    let res = query::<Res>(&q).await?;
    let mut count = 0usize;
    for repo in res.data.user.repositories.nodes {
        if repo.pullRequests.nodes.is_empty() {
            continue;
        }
        println!("{}", repo.name);
        for pr in repo.pullRequests.nodes {
            count += 1;
            println!("  #{} {} {} ", pr.number, pr.url, pr.title)
        }
    }
    println!("Count of PRs: {}", count);
    Ok(())
}

#[async_std::main]
async fn main() -> surf::Result<()> {
    let opt = Opt::from_args();
    check_prs(&opt.user).await?;
    Ok(())
}
