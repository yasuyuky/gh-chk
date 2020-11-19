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

pub async fn check(user: &str) -> surf::Result<()> {
    let v = json!({ "login": user });
    let q = json!({ "query": include_str!("../query.contributions.graphql"), "variables": v });
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
            let ck = d.color.get(31..33).unwrap_or_default();
            let c = colormap.get(ck).unwrap_or(&("black", 0xE6, 0xE8, 0xED));
            let s = format!("{:3}", d.contribution_count);
            print!("{} ", s.as_str().color(c.0).on_truecolor(c.1, c.2, c.3))
        }
        println!("");
    }
    println!("total contributions: {}", calendar.total_contributions);
    Ok(())
}
