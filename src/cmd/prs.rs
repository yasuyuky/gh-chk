use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::{Debug, Display};

use crate::cmd::prs::pull_request::PullRequest;
use crate::slug::Slug;
use crate::{config, graphql};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum RequestedReviewer {
    User { login: String },
    Team { name: String },
}

impl std::fmt::Display for RequestedReviewer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestedReviewer::User { login } => write!(f, "{}", login),
            RequestedReviewer::Team { name } => write!(f, "team:{}", name),
        }
    }
}

nestruct::nest! {
    #[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
    #[serde(rename_all = "camelCase")]
    PullRequest {
        repository: {
            name: String,
            owner: {
                login: String,
            }
        },
        id: String,
        number: usize,
        title: String,
        url: String,
        created_at: String,
        merge_state_status: crate::cmd::prs::MergeStateStatus,
        review_decision: crate::cmd::prs::ReviewDecision?,
        review_requests: {
            nodes: [{
                requested_reviewer: crate::cmd::prs::RequestedReviewer?,
            }]
        }
    }
}

impl pull_request::PullRequest {
    pub fn slug(&self) -> String {
        format!("{}/{}", self.repository.owner.login, self.repository.name)
    }
    pub fn numslug(&self) -> String {
        format!("#{} in {}", self.number, self.slug())
    }
    fn created_date(&self) -> &str {
        self.created_at
            .split('T')
            .next()
            .unwrap_or(&self.created_at)
    }
    fn review_requests(&self) -> String {
        if self.review_requests.nodes.is_empty() {
            String::default()
        } else if self.review_requests.nodes.len() == 1 {
            let name = &self.review_requests.nodes[0].requested_reviewer;
            format!("[r: {}]", name.as_ref().unwrap())
        } else {
            format!("[r: {}]", &self.review_requests.nodes.len())
        }
    }
    fn review_status(&self) -> String {
        match &self.review_decision {
            Some(rd) => format!("[{}]", rd),
            None => String::default(),
        }
    }
    fn colorized_string(&self) -> String {
        format!(
            "{:>6} {} {} {} {} {} {}",
            format!("#{}", self.number).bold(),
            self.merge_state_status.to_emoji(),
            self.merge_state_status.colorize(&self.url),
            self.title.bold(),
            self.review_decision
                .as_ref()
                .map(|rd| rd.colorize(&format!("[{}]", rd)))
                .unwrap_or_default(),
            self.review_requests(),
            format!("({})", self.created_date()).bright_black()
        )
    }
}

impl Display for pull_request::PullRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "#{} {} {} {} {} {} ({})",
            self.number,
            self.merge_state_status.to_emoji(),
            self.slug(),
            self.title,
            self.review_status(),
            self.review_requests(),
            self.created_date()
        )
    }
}

nestruct::nest! {
    #[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
    #[serde(rename_all = "camelCase")]
    Repository {
        name: String,
        pull_requests: {
            nodes: [ crate::cmd::prs::pull_request::PullRequest ]
        }
    }
}

nestruct::nest! {
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    Res {
        data: {
            repository_owner: {
                login: String,
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
                login: String,
                repository: crate::cmd::prs::repository::Repository
            }
        }
    }
}

nestruct::nest! {
    #[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
    #[serde(rename_all = "camelCase")]
    PrBodyRes {
        data: {
            repository_owner: {
                repository: {
                    pull_request: {
                        body_text: String,
                    }
                }
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MergeStateStatus {
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
    pub fn to_emoji(&self) -> String {
        match self {
            Self::Behind => "⏩",
            Self::Blocked => "🚫",
            Self::Clean => "✅",
            Self::Dirty => "⚠️",
            Self::Draft => "✏️",
            Self::HasHooks => "🪝",
            Self::Unknown => "❓",
            Self::Unstable => "❌",
        }
        .to_owned()
    }

    fn colorize(&self, s: &str) -> String {
        use colored::Colorize as _;
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

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
}

impl Display for ReviewDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Approved => "approved",
            Self::ChangesRequested => "changes requested",
            Self::ReviewRequired => "review required",
        };
        write!(f, "{}", s)
    }
}

impl ReviewDecision {
    fn colorize(&self, s: &str) -> String {
        use colored::Colorize as _;
        match self {
            Self::Approved => s.green(),
            Self::ChangesRequested => s.red(),
            Self::ReviewRequired => s.yellow(),
        }
        .to_string()
    }
}

#[derive(Deserialize)]
pub struct Diff {
    pub filename: String,
    pub additions: i64,
    pub deletions: i64,
    pub patch: Option<String>,
}

impl Display for Diff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut out = String::default();
        out += &format!(
            "=== {} (+{}, -{}) === \n",
            self.filename, self.additions, self.deletions
        );
        match self.patch {
            Some(ref p) => out.push_str(&format!("{}", p)),
            None => out.push_str(", (no textual diff available)"),
        };
        writeln!(f, "{}\n", out)
    }
}

nestruct::nest! {
    #[derive(serde::Deserialize, Clone)]
    Commit {
        sha: String,
        commit: {
            message: String,
            author: {
                name: String?,
                date: String?,
            }?,
        },
        parents: [{
            sha: String,
        }],
        author: {
            login: String?,
        }?,
    }
}

pub use commit::Commit;

impl Commit {
    pub fn summary(&self) -> String {
        let mut summary = self
            .commit
            .message
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if summary.len() > 80 {
            summary.truncate(77);
            summary.push_str("...");
        }
        summary
    }

    pub fn display_author(&self) -> Option<String> {
        if let Some(author) = self.author.as_ref()
            && let Some(login) = author.login.as_ref()
        {
            return Some(login.clone());
        }
        self.commit.author.as_ref().and_then(|a| a.name.clone())
    }

    pub fn display_date(&self) -> Option<String> {
        self.commit
            .author
            .as_ref()
            .and_then(|a| a.date.as_ref())
            .and_then(|date| date.split('T').next().map(str::to_string))
    }

    pub fn parent_shas(&self) -> impl Iterator<Item = &str> {
        self.parents.iter().map(|p| p.sha.as_str())
    }
}

#[derive(Clone)]
pub struct CommitGraphEntry {
    pub graph: String,
    pub short_sha: String,
    pub summary: String,
    pub author: Option<String>,
    pub date: Option<String>,
}

pub async fn fetch_pr_diffs(owner: &str, name: &str, number: usize) -> surf::Result<Vec<Diff>> {
    let path = format!("repos/{}/{}/pulls/{}/files", owner, name, number);
    let q: crate::rest::QueryMap = crate::rest::QueryMap::default();
    crate::rest::get(&path, 1, &q).await
}

pub async fn fetch_pr_commits(owner: &str, name: &str, number: usize) -> surf::Result<Vec<Commit>> {
    let path = format!("repos/{}/{}/pulls/{}/commits", owner, name, number);
    let q: crate::rest::QueryMap = crate::rest::QueryMap::default();
    crate::rest::get(&path, 1, &q).await
}

pub async fn fetch_pr_body(owner: &str, name: &str, number: usize) -> surf::Result<String> {
    let v = json!({ "login": owner, "name": name, "number": number });
    let q = json!({"query": include_str!("../query/prs.graphql"), "operationName": "GetPrBody", "variables": v});
    let res = graphql::query::<pr_body_res::PrBodyRes>(&q).await?;
    Ok(res.data.repository_owner.repository.pull_request.body_text)
}

pub async fn merge_pr(pr_id: &str) -> surf::Result<()> {
    let v = json!({ "pullRequestId": pr_id });
    let q = json!({ "query": include_str!("../query/prs.graphql"), "operationName": "MergePullRequest", "variables": v });
    crate::graphql::query::<serde_json::Value>(&q).await?;
    Ok(())
}

pub async fn check(slugs: Vec<String>, merge: bool) -> surf::Result<()> {
    let slugs = if slugs.is_empty() {
        vec![crate::cmd::viewer::get().await?]
    } else {
        slugs
    };

    if matches!(config::FORMAT.get(), Some(config::Format::Json)) {
        let specs: Vec<Slug> = slugs.iter().map(|s| Slug::from(s.as_str())).collect();
        let prs = fetch_prs(&specs).await?;
        println!("{}", serde_json::to_string_pretty(&prs).unwrap());
        return Ok(());
    }

    for slug in slugs {
        println!("{}", slug.bright_blue());
        let slug = Slug::from(slug.as_str());
        let prs = fetch_prs(&vec![slug]).await?;
        for pr in &prs {
            println!("{}", pr.colorized_string());
            if merge && pr.merge_state_status == MergeStateStatus::Clean {
                println!("🔄 Merging PR #{}", pr.number);
                merge_pr(&pr.id).await?;
                println!("✅ Merged PR #{}", pr.number);
            }
        }
    }
    Ok(())
}

pub async fn fetch_prs(specs: &Vec<Slug>) -> surf::Result<Vec<PullRequest>> {
    let mut all_prs: Vec<PullRequest> = Vec::new();
    for spec in specs {
        match spec {
            Slug::Owner(owner) => all_prs.append(&mut fetch_owner_prs(owner).await?),
            Slug::Repo { owner, name } => all_prs.append(&mut fetch_repo_prs(owner, name).await?),
        }
    }
    Ok(all_prs)
}

async fn fetch_owner_prs(owner: &str) -> surf::Result<Vec<PullRequest>> {
    let v = json!({ "login": owner });
    let q = json!({ "query": include_str!("../query/prs.graphql"), "operationName": "GetOwnerPrs", "variables": v });
    let res = graphql::query::<res::Res>(&q).await?;
    let mut prs = Vec::new();
    for repo in res.data.repository_owner.repositories.nodes {
        prs.extend(repo.pull_requests.nodes);
    }
    Ok(prs)
}

async fn fetch_repo_prs(owner: &str, name: &str) -> surf::Result<Vec<PullRequest>> {
    let v = json!({ "login": owner, "name": name });
    let q = json!({ "query": include_str!("../query/prs.graphql"), "operationName": "GetRepoPrs", "variables": v });
    let res = graphql::query::<repo_res::RepoRes>(&q).await?;
    Ok(res.data.repository_owner.repository.pull_requests.nodes)
}

pub async fn approve_pr(pr_id: &str) -> surf::Result<()> {
    let v = json!({ "pullRequestId": pr_id });
    let q = json!({ "query": include_str!("../query/prs.graphql"), "operationName": "ApprovePullRequest", "variables": v });
    graphql::query::<serde_json::Value>(&q).await?;
    Ok(())
}
