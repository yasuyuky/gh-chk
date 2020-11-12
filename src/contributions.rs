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
#[allow(non_snake_case)]
#[derive(Deserialize)]
struct ContributionCalendar {
    totalContributions: usize,
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
    let calendar = res.data.user.contributionsCollection.contributionCalendar;

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
        print!("{}: ", week.firstDay);
        for d in week.contributionDays {
            let ck = d.color.get(31..33).unwrap_or_default();
            let c = colormap.get(ck).unwrap_or(&("black", 0xE6, 0xE8, 0xED));
            let s = format!("{:3}", d.contributionCount);
            print!("{} ", s.as_str().color(c.0).on_truecolor(c.1, c.2, c.3))
        }
        println!("");
    }
    println!("total contributions: {}", calendar.totalContributions);
    Ok(())
}
