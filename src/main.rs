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

fn build_q(user: &str) -> String {
    let query = "query ($login: String!) {
        user(login: $login) {
          repositories(first: 100, affiliations: OWNER) {
            nodes {
              name
              pullRequests(first: 100, states: OPEN) {
                nodes {
                  number
                  title
                  url
                }
              }
            }
          }
        }
      }";
    json!({
        "query": query,
        "variables": {"login": user}
    })
    .to_string()
}

#[async_std::main]
async fn main() -> surf::Result<()> {
    let opt = Opt::from_args();
    let q = build_q(&opt.user);
    let res = query::<Res>(&q).await?;
    for repo in res.data.user.repositories.nodes {
        if repo.pullRequests.nodes.is_empty() {
            continue;
        }
        println!("{}", repo.name);
        for pr in repo.pullRequests.nodes {
            println!("  #{} {} {} ", pr.number, pr.url, pr.title)
        }
    }
    Ok(())
}