use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Serialize, Deserialize)]
struct Res {
    data: Data,
}
#[allow(non_snake_case)]
#[derive(Serialize, Deserialize)]
struct Data {
    repositoryOwner: RepositoriesOwner,
}
#[derive(Serialize, Deserialize)]
struct RepositoriesOwner {
    repositories: RepositoryConnection,
}
#[derive(Serialize, Deserialize)]
struct RepositoryConnection {
    nodes: Vec<Repository>,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize)]
struct Repository {
    name: String,
    issues: IssuesConnection,
}
#[derive(Serialize, Deserialize)]
struct IssuesConnection {
    nodes: Vec<PullRequest>,
}
#[derive(Serialize, Deserialize)]
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
    let q = json!({ "query": include_str!("../query/issues.graphql"), "variables": v });
    let res = crate::graphql::query::<Res>(&q).await?;
    let mut count = 0usize;
    for repo in res.data.repositoryOwner.repositories.nodes {
        if repo.issues.nodes.is_empty() {
            continue;
        }
        println!("{}", repo.name.cyan());
        for issue in repo.issues.nodes {
            count += 1;
            println!("  #{} {} {} ", issue.number, issue.url, issue.title)
        }
    }
    println!("Count of Issues: {}", count);
    Ok(())
}
