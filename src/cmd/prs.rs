use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::Display;

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

#[derive(Serialize, Deserialize)]
struct SingleRepoRes {
    data: SingleRepoData,
}
#[allow(non_snake_case)]
#[derive(Serialize, Deserialize)]
struct SingleRepoData {
    repositoryOwner: RepositoryOwner,
}
#[derive(Serialize, Deserialize)]
struct RepositoryOwner {
    repository: Repository,
}

#[allow(non_snake_case)]
#[derive(Serialize, Deserialize)]
struct Repository {
    name: String,
    pullRequests: PullRequestsConnection,
}
#[derive(Serialize, Deserialize)]
struct PullRequestsConnection {
    nodes: Vec<PullRequest>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequest {
    pub number: usize,
    pub title: String,
    pub url: String,
    pub merge_state_status: MergeStateStatus,
}

impl Display for PullRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = format!(
            "{:>6} {} {} {}",
            format!("#{}", self.number).bold(),
            self.merge_state_status.to_emoji(),
            self.url,
            self.title.bold()
        );
        write!(f, "{}", self.merge_state_status.colorize(&s))
    }
}

#[derive(Serialize, Deserialize, Debug)]
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

impl MergeStateStatus {
    fn to_emoji(&self) -> String {
        match self {
            MergeStateStatus::Behind => "⏩",
            MergeStateStatus::Blocked => "🚫",
            MergeStateStatus::Clean => "✅",
            MergeStateStatus::Dirty => "⚠️ ",
            MergeStateStatus::Draft => "✏️ ",
            MergeStateStatus::HasHooks => "🪝",
            MergeStateStatus::Unknown => "❓",
            MergeStateStatus::Unstable => "❌",
        }
        .to_owned()
    }

    fn colorize(&self, s: &str) -> String {
        match self {
            MergeStateStatus::Behind => s.yellow(),
            MergeStateStatus::Blocked => s.red(),
            MergeStateStatus::Clean => s.green(),
            MergeStateStatus::Dirty => s.yellow(),
            MergeStateStatus::Draft => s.white(),
            MergeStateStatus::HasHooks => s.yellow(),
            MergeStateStatus::Unknown => s.magenta(),
            MergeStateStatus::Unstable => s.yellow(),
        }
        .to_string()
    }
}

pub async fn check(slugs: Vec<String>) -> surf::Result<()> {
    let slugs = if slugs.is_empty() {
        vec![crate::cmd::viewer::get().await?]
    } else {
        slugs
    };
    for slug in slugs {
        let vs: Vec<String> = slug.split('/').map(String::from).collect();
        match vs.len() {
            1 => check_owner(&vs[0]).await?,
            2 => check_repo(&vs[0], &vs[1]).await?,
            _ => panic!("unknown slug format"),
        }
    }
    Ok(())
}

async fn check_owner(owner: &str) -> surf::Result<()> {
    let v = json!({ "login": owner });
    let q = json!({ "query": include_str!("../query/prs.graphql"), "variables": v });
    let res = crate::graphql::query::<Res>(&q).await?;
    match crate::config::FORMAT.get() {
        Some(&crate::config::Format::Json) => println!("{}", serde_json::to_string_pretty(&res)?),
        _ => print_owner_text(&res),
    }
    Ok(())
}

fn print_owner_text(res: &Res) {
    let mut count = 0usize;
    for repo in &res.data.repositoryOwner.repositories.nodes {
        if repo.pullRequests.nodes.is_empty() {
            continue;
        }
        println!("{}", repo.name.cyan());
        for pr in &repo.pullRequests.nodes {
            count += 1;
            println!("{}", pr);
        }
    }
    println!("Count of PRs: {}", count);
}

async fn check_repo(owner: &str, name: &str) -> surf::Result<()> {
    let v = json!({ "login": owner, "name": name });
    let q = json!({ "query": include_str!("../query/prs.repo.graphql"), "variables": v });
    let res = crate::graphql::query::<SingleRepoRes>(&q).await?;
    match crate::config::FORMAT.get() {
        Some(&crate::config::Format::Json) => println!("{}", serde_json::to_string_pretty(&res)?),
        _ => print_repo_text(&res),
    }
    Ok(())
}

fn print_repo_text(res: &SingleRepoRes) {
    let mut count = 0usize;
    for pr in &res.data.repositoryOwner.repository.pullRequests.nodes {
        count += 1;
        println!("{}", pr);
    }
    println!("Count of PRs: {}", count);
}
