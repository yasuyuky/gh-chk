use serde::Deserialize;
use serde_json::json;


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
    pub number: usize,
    pub title: String,
    pub url: String,
}

pub async fn check_prs(user: &str) -> surf::Result<()> {
    let v = json!({ "login": user });
    let q = json!({ "query": include_str!("query.user.repo.pr.graphql"), "variables": v });
    let res = crate::query::<Res>(&q).await?;
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
