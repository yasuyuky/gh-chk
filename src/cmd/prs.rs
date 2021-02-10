use colored::Colorize;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Res {
    data: Data,
}
#[allow(non_snake_case)]
#[derive(Deserialize)]
struct Data {
    repositoryOwner: RepositoryOwner,
}
#[derive(Deserialize)]
struct RepositoryOwner {
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
    pub number: usize,
    pub title: String,
    pub url: String,
}

pub async fn check(slug: Option<String>) -> surf::Result<()> {
    let slug = slug.unwrap_or(crate::cmd::viewer::get().await?);
    let vs: Vec<String> = slug.split('/').map(String::from).collect();
    match vs.len() {
        1 => check_owner(&vs[0]).await,
        _ => panic!("unknown slug format"),
    }
}

async fn check_owner(owner: &str) -> surf::Result<()> {
    let v = json!({ "login": owner });
    let q = json!({ "query": include_str!("../query/prs.graphql"), "variables": v });
    query_prs(&q).await
}

async fn query_prs(q: &serde_json::Value) -> surf::Result<()> {
    let res = crate::graphql::query::<Res>(&q).await?;
    let mut count = 0usize;
    for repo in res.data.repositoryOwner.repositories.nodes {
        if repo.pullRequests.nodes.is_empty() {
            continue;
        }
        println!("{}", repo.name.cyan());
        for pr in repo.pullRequests.nodes {
            count += 1;
            println!("  #{} {} {} ", pr.number, pr.url, pr.title)
        }
    }
    println!("Count of PRs: {}", count);
    Ok(())
}
