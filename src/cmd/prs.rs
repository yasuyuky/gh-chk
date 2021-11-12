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
    repositoryOwner: RepositoriesOwner,
}
#[derive(Deserialize)]
struct RepositoriesOwner {
    repositories: RepositoryConnection,
}
#[derive(Deserialize)]
struct RepositoryConnection {
    nodes: Vec<Repository>,
}

#[derive(Deserialize)]
struct SingleRepoRes {
    data: SingleRepoData,
}
#[allow(non_snake_case)]
#[derive(Deserialize)]
struct SingleRepoData {
    repositoryOwner: RepositoryOwner,
}
#[derive(Deserialize)]
struct RepositoryOwner {
    repository: Repository,
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
#[serde(rename_all = "camelCase")]
struct PullRequest {
    pub number: usize,
    pub title: String,
    pub url: String,
    pub merge_state_status: MergeStateStatus,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum MergeStateStatus {
    Behind,
    Blocked,
    Clean,
    Dirty,
    Draft,
    HasHooks,
    Unknown,
    Unstable,
}

pub async fn check(slug: Option<String>) -> surf::Result<()> {
    let slug = slug.unwrap_or(crate::cmd::viewer::get().await?);
    let vs: Vec<String> = slug.split('/').map(String::from).collect();
    match vs.len() {
        1 => check_owner(&vs[0]).await,
        2 => check_repo(&vs[0], &vs[1]).await,
        _ => panic!("unknown slug format"),
    }
}

async fn check_owner(owner: &str) -> surf::Result<()> {
    let v = json!({ "login": owner });
    let q = json!({ "query": include_str!("../query/prs.graphql"), "variables": v });
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

async fn check_repo(owner: &str, name: &str) -> surf::Result<()> {
    let v = json!({ "login": owner, "name": name });
    let q = json!({ "query": include_str!("../query/prs.repo.graphql"), "variables": v });
    let res = crate::graphql::query::<SingleRepoRes>(&q).await?;
    let mut count = 0usize;
    for pr in res.data.repositoryOwner.repository.pullRequests.nodes {
        count += 1;
        println!("  #{} {} {} ", pr.number, pr.url, pr.title)
    }
    println!("Count of PRs: {}", count);
    Ok(())
}
