use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::{Debug, Display};

use crate::cmd::prs::pull_request::PullRequest;
use crate::graphql;
use crate::slug::Slug;

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
    #[derive(serde::Serialize, serde::Deserialize, Clone)]
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
        body_text: String,
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
    fn review_status(&self) -> String {
        match &self.review_decision {
            Some(rd) => {
                let label = rd.to_label();
                let bracketed = format!("[{}]", label);
                rd.colorize(&bracketed)
            }
            None => String::default(),
        }
    }
}

impl Display for pull_request::PullRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let review_str = match &self.review_decision {
            Some(ReviewDecision::Approved) => " [approved]".to_string(),
            Some(ReviewDecision::ChangesRequested) => " [changes requested]".to_string(),
            Some(ReviewDecision::ReviewRequired) => " [review required]".to_string(),
            None => String::default(),
        };
        let reviewers_str = if self.review_requests.nodes.is_empty() {
            String::default()
        } else {
            format!(
                " 👥 {}",
                extract_reviewer_names(&self.review_requests).join(", ")
            )
        };
        write!(
            f,
            "#{} {} {} {}{}{} ({})",
            self.number,
            self.merge_state_status.to_emoji(),
            self.slug(),
            self.title,
            review_str,
            reviewers_str,
            self.created_date()
        )
    }
}

impl Debug for pull_request::PullRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let review = match &self.review_decision {
            Some(rd) => {
                let label = rd.to_label();
                let bracketed = format!("[{}]", label);
                rd.colorize(&bracketed)
            }
            None => String::default(),
        };
        let review_sep = if review.is_empty() { "" } else { " " };
        let s = format!(
            "{:>6} {} {} {}{}{} {}",
            format!("#{}", self.number).bold(),
            self.merge_state_status.to_emoji(),
            self.url,
            self.title.bold(),
            review_sep,
            review,
            format!("({})", self.created_date()).bright_black()
        );
        write!(f, "{}", self.merge_state_status.colorize(&s))
    }
}

fn extract_reviewer_names(
    review_requests: &pull_request::review_requests::ReviewRequests,
) -> Vec<String> {
    review_requests
        .nodes
        .iter()
        .filter_map(|node| node.requested_reviewer.as_ref().map(ToString::to_string))
        .collect()
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

impl ReviewDecision {
    pub fn to_label(&self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::ChangesRequested => "changes requested",
            Self::ReviewRequired => "review required",
        }
    }

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

pub async fn merge_pr(pr_id: &str) -> surf::Result<()> {
    let v = json!({ "pullRequestId": pr_id });
    let q = json!({ "query": include_str!("../query/merge.pr.graphql"), "variables": v });
    crate::graphql::query::<serde_json::Value>(&q).await?;
    Ok(())
}

pub async fn check(slugs: Vec<String>, merge: bool) -> surf::Result<()> {
    let slugs = if slugs.is_empty() {
        vec![crate::cmd::viewer::get().await?]
    } else {
        slugs
    };

    for slug in slugs {
        println!("{}", slug.bright_blue());
        let slug = Slug::from(slug.as_str());
        let prs = fetch_prs(&vec![slug]).await?;
        for pr in &prs {
            println!("{:?}", pr);
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
    let q = json!({ "query": include_str!("../query/approve.pr.graphql"), "variables": v });
    graphql::query::<serde_json::Value>(&q).await?;
    Ok(())
}
