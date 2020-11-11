use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Res {
    data: Data,
}
#[derive(Deserialize)]
struct Data {
    user: User,
}
#[allow(non_snake_case)]
#[derive(Deserialize)]
struct User {
    contributionsCollection: ContributionCollection,
}
#[allow(non_snake_case)]
#[derive(Deserialize)]
struct ContributionCollection {
    contributionCalendar: ContributionCalendar,
}
#[derive(Deserialize)]
struct ContributionCalendar {
    colors: Vec<String>,
    weeks: Vec<Week>,
}
#[allow(non_snake_case)]
#[derive(Deserialize)]
struct Week {
    firstDay: String,
    contributionDays: Vec<ContributionDay>,
}

#[allow(non_snake_case)]
#[derive(Deserialize)]
struct ContributionDay {
    color: String,
    contributionCount: usize,
}

pub async fn check(user: &str) -> surf::Result<()> {
    let v = json!({ "login": user });
    let q = json!({ "query": include_str!("query.contributions.graphql"), "variables": v });
    let res = crate::query::<Res>(&q).await?;
    for week in res
        .data
        .user
        .contributionsCollection
        .contributionCalendar
        .weeks
    {
        print!("{}: ", week.firstDay);
        for d in week.contributionDays {
            print!("{:2} ", d.contributionCount)
        }
        println!("");
    }
    Ok(())
}
