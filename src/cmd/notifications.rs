use std::collections::HashMap;

use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::json;

nestruct::nest! {
    #[derive(serde::Serialize, serde::Deserialize)]
    Notification {
        id: String,
        repository: {
            full_name: String
        },
        subject: {
            #[serde(rename = "type")]
            ntype: String,
            title: String,
            url: Option<String>,
        },
        reason: String,
        #[serde(deserialize_with = "time::serde::iso8601::deserialize")]
        updated_at: time::OffsetDateTime,
    }
}

pub async fn list(read: bool) -> surf::Result<()> {
    let mut res = Vec::new();
    let mut page = 1;
    while let Ok(mut page_res) = list_page(page).await {
        if page_res.is_empty() {
            break;
        }
        res.append(&mut page_res);
        page += 1;
    }
    match crate::config::FORMAT.get() {
        Some(&crate::config::Format::Json) => println!("{}", serde_json::to_string_pretty(&res)?),
        _ => print_text(&res, read).await,
    }
    Ok(())
}

pub async fn list_page(page: usize) -> surf::Result<Vec<notification::Notification>> {
    let q = HashMap::new();
    let res = crate::rest::get::<notification::Notification>("notifications", page, &q).await?;
    Ok(res)
}

async fn print_text(res: &[notification::Notification], read: bool) {
    for n in res {
        let status = match &n.subject.url {
            Some(url) => get_status(url).await.unwrap_or_default(),
            None => String::default(),
        };
        println!(
            "{:10} {:12} {:11} {:6} {} {} {} {}",
            n.id.black(),
            n.reason.magenta(),
            n.subject.ntype.yellow(),
            status,
            n.updated_at.date(),
            n.repository.full_name.cyan(),
            n.subject.title,
            n.subject.url.clone().unwrap_or_default().green(),
        );
        if read {
            match status.as_str() {
                "MERGED" | "CLOSED" => {
                    let path = "notifications/threads/".to_owned() + &n.id;
                    let _ = crate::rest::patch(&path).await;
                }
                _ => {}
            }
        }
    }
    println!("# count: {}", res.len());
}

#[derive(Serialize, Deserialize)]
struct Res {
    data: Data,
}

#[derive(Serialize, Deserialize)]
struct Data {
    resource: Resource,
}

#[derive(Serialize, Deserialize)]
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
