use colored::Colorize;
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
#[serde(rename_all = "camelCase")]
#[derive(Deserialize)]
struct User {
    contributions_collection: ContributionCollection,
}
#[serde(rename_all = "camelCase")]
#[derive(Deserialize)]
struct ContributionCollection {
    contribution_calendar: ContributionCalendar,
}
#[serde(rename_all = "camelCase")]
#[derive(Deserialize)]
struct ContributionCalendar {
    total_contributions: usize,
    weeks: Vec<Week>,
}
#[serde(rename_all = "camelCase")]
#[derive(Deserialize)]
struct Week {
    first_day: String,
    contribution_days: Vec<ContributionDay>,
}
#[serde(rename_all = "camelCase")]
#[derive(Deserialize)]
struct ContributionDay {
    color: String,
    contribution_count: usize,
}

pub async fn check(user: Option<String>) -> surf::Result<()> {
    let user = user.unwrap_or(crate::cmd::viewer::get().await?);
    let var = json!({ "login": user });
    let q = json!({ "query": include_str!("../query/contributions.graphql"), "variables": var });
    let res = crate::graphql::query::<Res>(&q).await?;
    let calendar = res.data.user.contributions_collection.contribution_calendar;

    for week in calendar.weeks {
        print!("{}: ", week.first_day);
        for day in week.contribution_days {
            let r = u8::from_str_radix(day.color.get(1..3).unwrap_or_default(), 16)?;
            let g = u8::from_str_radix(day.color.get(3..5).unwrap_or_default(), 16)?;
            let b = u8::from_str_radix(day.color.get(5..7).unwrap_or_default(), 16)?;
            let cnt = format!("{:3}", day.contribution_count);
            print!("{} ", cnt.as_str().color("black").on_truecolor(r, g, b))
        }
        println!();
    }
    println!("total contributions: {}", calendar.total_contributions);
    Ok(())
}
