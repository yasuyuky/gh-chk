use colored::Colorize;
use serde::Deserialize;

#[derive(Deserialize)]
struct Notification {
    id: String,
    repository: Repository,
    subject: Subject,
    reason: String,
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
}

pub async fn check() -> surf::Result<()> {
    let res = crate::rest::get::<Notification>("notifications", |i| i < 1).await?;
    for n in &res {
        println!(
            "{} {} {} {} {}",
            n.id,
            n.reason.magenta(),
            n.repository.full_name,
            n.subject.ntype.cyan(),
            n.subject.title
        )
    }
    println!("# total count: {}", res.len());
    Ok(())
}
