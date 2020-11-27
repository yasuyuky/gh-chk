use colored::Colorize;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

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
    let v = json!({ "login": user });
    let q = json!({ "query": include_str!("../query/contributions.graphql"), "variables": v });
    let res = crate::graphql::query::<Res>(&q).await?;
    let calendar = res.data.user.contributions_collection.contribution_calendar;

    let colormap: HashMap<&str, (&str, u8, u8, u8)> = [
        ("L1", ("black", 0x8C, 0xE7, 0x98)),
        ("L2", ("black", 0x38, 0xBC, 0x51)),
        ("L3", ("white", 0x29, 0x94, 0x3D)),
        ("L4", ("white", 0x1B, 0x5D, 0x2B)),
    ]
    .iter()
    .cloned()
    .collect();

    for week in calendar.weeks {
        print!("{}: ", week.first_day);
        for d in week.contribution_days {
            let r = u8::from_str_radix(d.color.get(1..3).unwrap_or_default(), 16)?;
            let g = u8::from_str_radix(d.color.get(3..5).unwrap_or_default(), 16)?;
            let b = u8::from_str_radix(d.color.get(5..7).unwrap_or_default(), 16)?;
            let s = format!("{:3}", d.contribution_count);
            print!("{} ", s.as_str().color("black").on_truecolor(r, g, b))
        }
        println!("");
    }
    println!("total contributions: {}", calendar.total_contributions);
    Ok(())
}
