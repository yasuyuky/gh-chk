use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::Display;

nestruct::nest! {
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    Repository {
        name: String,
        #[serde(rename="pullRequests")]
        pull_requests: {
            nodes: [{
                number: usize,
                title: String,
                url: String,
                #[serde(rename="mergeStateStatus")]
                merge_state_status: #[serde(rename_all = "SCREAMING_SNAKE_CASE")] {
                    Behind,
                    Blocked,
                    Clean,
                    Dirty,
                    Draft,
                    HasHooks,
                    Unknown,
                    Unstable,
                },
            }]
        }
    }
}

nestruct::nest! {
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    Res {
        data: {
            repository_owner: {
                repositories: {
                    nodes: [ crate::cmd::prs::repository::Repository ]
                }
            }
        }
    }
}

nestruct::nest! {
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    RepoRes {
        data: {
            repository_owner: {
                repository: crate::cmd::prs::repository::Repository
            }
        }
    }
}

impl Display for repository::pull_requests::nodes::Nodes {
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

impl repository::pull_requests::nodes::merge_state_status::MergeStateStatus {
    fn to_emoji(&self) -> String {
        match self {
            Self::Behind => "⏩",
            Self::Blocked => "🚫",
            Self::Clean => "✅",
            Self::Dirty => "⚠️ ",
            Self::Draft => "✏️ ",
            Self::HasHooks => "🪝",
            Self::Unknown => "❓",
            Self::Unstable => "❌",
        }
        .to_owned()
    }

    fn colorize(&self, s: &str) -> String {
        match self {
            Self::Behind => s.yellow(),
            Self::Blocked => s.red(),
            Self::Clean => s.green(),
            Self::Dirty => s.yellow(),
            Self::Draft => s.white(),
            Self::HasHooks => s.yellow(),
            Self::Unknown => s.magenta(),
            Self::Unstable => s.yellow(),
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
        println!("{}", slug.bright_blue());
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
    let res = crate::graphql::query::<res::Res>(&q).await?;
    match crate::config::FORMAT.get() {
        Some(&crate::config::Format::Json) => println!("{}", serde_json::to_string_pretty(&res)?),
        _ => print_owner_text(&res),
    }
    Ok(())
}

fn print_owner_text(res: &res::Res) {
    let mut count = 0usize;
    for repo in &res.data.repository_owner.repositories.nodes {
        if repo.pull_requests.nodes.is_empty() {
            continue;
        }
        println!("{}", repo.name.cyan());
        for pr in &repo.pull_requests.nodes {
            count += 1;
            println!("{}", pr);
        }
    }
    println!("Count of PRs: {}", count);
}

async fn check_repo(owner: &str, name: &str) -> surf::Result<()> {
    let v = json!({ "login": owner, "name": name });
    let q = json!({ "query": include_str!("../query/prs.repo.graphql"), "variables": v });
    let res = crate::graphql::query::<repo_res::RepoRes>(&q).await?;
    match crate::config::FORMAT.get() {
        Some(&crate::config::Format::Json) => println!("{}", serde_json::to_string_pretty(&res)?),
        _ => print_repo_text(&res),
    }
    Ok(())
}

fn print_repo_text(res: &repo_res::RepoRes) {
    let mut count = 0usize;
    for pr in &res.data.repository_owner.repository.pull_requests.nodes {
        count += 1;
        println!("{}", pr);
    }
    println!("Count of PRs: {}", count);
}
