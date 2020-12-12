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
    url: String,
}

pub async fn list(page: usize) -> surf::Result<()> {
    let res = crate::rest::get::<Notification>("notifications", page).await?;
    for n in &res {
        println!(
            "{:10} {:10} {:11} {} {}",
            n.id.black(),
            n.reason.magenta(),
            n.subject.ntype.yellow(),
            n.repository.full_name.cyan(),
            n.subject.title
        )
    }
    println!("# count: {}", res.len());
    Ok(())
}
