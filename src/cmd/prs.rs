use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::Display;

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
    pub fn display_line(&self) -> String {
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
        let created_date = self
            .created_at
            .split('T')
            .next()
            .unwrap_or(&self.created_at)
            .to_string();
        format!(
            "#{} {} {} {}{}{} ({})",
            self.number,
            self.merge_state_status.to_emoji(),
            self.slug(),
            self.title,
            review_str,
            reviewers_str,
            created_date
        )
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

impl Display for pull_request::PullRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let created_date = self
            .created_at
            .split('T')
            .next()
            .unwrap_or(&self.created_at)
            .to_string();
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
            format!("({})", created_date).bright_black()
        );
        write!(f, "{}", self.merge_state_status.colorize(&s))
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
            Self::Dirty => "⚠️ ",
            Self::Draft => "✏️ ",
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
        let vs: Vec<String> = slug.split('/').map(String::from).collect();
        match vs.len() {
            1 => check_owner(&vs[0], merge).await?,
            2 => check_repo(&vs[0], &vs[1], merge).await?,
            _ => panic!("unknown slug format"),
        }
    }
    Ok(())
}

async fn check_owner(owner: &str, merge: bool) -> surf::Result<()> {
    let v = json!({ "login": owner });
    let q = json!({ "query": include_str!("../query/prs.graphql"), "operationName": "GetOwnerPrs", "variables": v });
    let res = crate::graphql::query::<res::Res>(&q).await?;
    match crate::config::FORMAT.get() {
        Some(&crate::config::Format::Json) => println!("{}", serde_json::to_string_pretty(&res)?),
        _ => print_owner_text(&res, merge).await?,
    }
    Ok(())
}

async fn print_owner_text(res: &res::Res, merge: bool) -> surf::Result<()> {
    let mut count = 0usize;
    for repo in &res.data.repository_owner.repositories.nodes {
        if repo.pull_requests.nodes.is_empty() {
            continue;
        }
        println!("{}", repo.name.cyan());
        for pr in &repo.pull_requests.nodes {
            count += 1;
            println!("{pr}");
            if merge && pr.merge_state_status == MergeStateStatus::Clean {
                println!("🔄 Merging PR #{}", pr.number);
                merge_pr(&pr.id).await?;
                println!("✅ Merged PR #{}", pr.number);
            }
        }
    }
    println!("Count of PRs: {count}");
    Ok(())
}

async fn check_repo(owner: &str, name: &str, merge: bool) -> surf::Result<()> {
    let v = json!({ "login": owner, "name": name });
    let q = json!({ "query": include_str!("../query/prs.graphql"), "operationName": "GetRepoPrs", "variables": v });
    let res = crate::graphql::query::<repo_res::RepoRes>(&q).await?;
    match crate::config::FORMAT.get() {
        Some(&crate::config::Format::Json) => println!("{}", serde_json::to_string_pretty(&res)?),
        _ => print_repo_text(&res, merge).await?,
    }
    Ok(())
}

async fn print_repo_text(res: &repo_res::RepoRes, merge: bool) -> surf::Result<()> {
    let mut count = 0usize;
    for pr in &res.data.repository_owner.repository.pull_requests.nodes {
        count += 1;
        println!("{pr}");
        if merge && pr.merge_state_status == MergeStateStatus::Clean {
            println!("🔄 Merging PR #{}", pr.number);
            merge_pr(&pr.id).await?;
            println!("✅ Merged PR #{}", pr.number);
        }
    }
    println!("Count of PRs: {count}");
    Ok(())
}
