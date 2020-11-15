use colored::Colorize;
use serde::Deserialize;

#[derive(Deserialize)]
struct Notification {
    repository: Repository,
    subject: Subject,
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
    let res = crate::rest::get::<Notification>("notifications").await?;
    for n in res {
        println!(
            "{} {} {}",
            n.repository.full_name,
            n.subject.ntype.cyan(),
            n.subject.title
        )
    }

    Ok(())
}
