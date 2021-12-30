use colored::Colorize;
use serde::Deserialize;
use serde_json::json;
use toml::value::Datetime;

#[derive(Deserialize)]
struct Res {
    data: Data,
}
#[derive(Deserialize)]
struct Data {
    repository: Repository,
}
#[derive(Deserialize)]
struct Repository {
    issue: Issue,
}

#[allow(non_snake_case)]
#[derive(Deserialize)]
struct Issue {
    number: usize,
    title: String,
    timelineItems: TimelineItemsConnection,
}

#[derive(Deserialize)]
struct TimelineItemsConnection {
    nodes: Vec<TimelineItem>,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct TimelineItem {
    __typename: TimelineItemType,
    createdAt: String,
    assignee: Assignee,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
enum TimelineItemType {
    AssignedEvent,
    UnassignedEvent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Assignee {
    User { login: String, name: String },
    Unknown,
}

pub async fn track(slug: &str, num: usize) -> surf::Result<()> {
    let vs: Vec<String> = slug.split('/').map(String::from).collect();
    match vs.len() {
        2 => track_issue(&vs[0], &vs[1], num).await,
        _ => panic!("unknown slug format"),
    }
}

async fn track_issue(owner: &str, name: &str, num: usize) -> surf::Result<()> {
    let v = json!({ "owner": owner, "name": name, "number": num });
    let q = json!({ "query": include_str!("../query/trackassignees.graphql"), "variables": v });
    let res: Res = crate::graphql::query::<Res>(&q).await?;
    let (mut maxcount, mut count) = (0isize, 0isize);
    println!(
        "{}/{}#{} {}",
        owner.cyan(),
        name.cyan(),
        num,
        res.data.repository.issue.title.yellow()
    );
    for item in res.data.repository.issue.timelineItems.nodes {
        count += if item.__typename == TimelineItemType::AssignedEvent {
            1
        } else {
            -1
        };
        maxcount = maxcount.max(count);
        println!(
            "  {:?} {} {:?}",
            item.__typename, item.createdAt, item.assignee
        );
    }
    println!("Count of Max assignees: {}", maxcount);
    Ok(())
}
