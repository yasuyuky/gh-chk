use chrono::{DateTime, FixedOffset};
use colored::Colorize;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Notification {
    id: String,
    repository: Repository,
    subject: Subject,
    reason: String,
    updated_at: DateTime<FixedOffset>,
}
#[derive(Deserialize)]
struct Repository {
    full_name: String,
}
#[derive(Deserialize)]
struct Subject {
    #[serde(rename = "type")]
    ntype: String,
    title: String,
    url: Option<String>,
}

pub async fn list(page: usize) -> surf::Result<()> {
    let res = crate::rest::get::<Notification>("notifications", page).await?;
    for n in &res {
        let status = match &n.subject.url {
            Some(url) => get_status(url).await.unwrap_or_default(),
            None => String::default(),
        };
        println!(
            "{:10} {:10} {:11} {:6} {} {} {} {}",
            n.id.black(),
            n.reason.magenta(),
            n.subject.ntype.yellow(),
            status,
            n.updated_at.naive_local().date(),
            n.repository.full_name.cyan(),
            n.subject.title,
            n.subject.url.clone().unwrap_or_default().green(),
        )
    }
    println!("# count: {}", res.len());
    Ok(())
}

#[derive(Deserialize)]
struct Res {
    data: Data,
}

#[derive(Deserialize)]
struct Data {
    resource: Resource,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Resource {
    IssueStatus { issue_state: String },
    PullRequestStatus { pr_state: String },
}

async fn get_status(api_url: &str) -> surf::Result<String> {
    let url = api_url
        .replace("api.github.com/repos", "github.com")
        .replace("/pulls/", "/pull/");
    let v = json!({ "url": url });
    let q = json!({ "query": include_str!("../query/resource.status.graphql"), "variables": v });
    let res = crate::graphql::query::<Res>(&q).await?;
    Ok(match res.data.resource {
        Resource::IssueStatus { issue_state } => issue_state,
        Resource::PullRequestStatus { pr_state } => pr_state,
    })
}
