use colored::Colorize;
use serde_json::json;

nestruct::nest! {
    #[derive(serde::Deserialize, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    Res {
        data: {
            user: {
                contributions_collection: {
                    contribution_calendar: {
                        total_contributions: usize,
                        weeks: [{
                            first_day: String,
                            contribution_days: [{
                                color: String,
                                contribution_count: usize,
                            }]
                        }]
                    }
                }
            }
        }
    }
}

pub async fn check(user: Option<String>) -> surf::Result<()> {
    let user = user.unwrap_or(crate::cmd::viewer::get().await?);
    let var = json!({ "login": user });
    let q = json!({ "query": include_str!("../query/contributions.graphql"), "variables": var });
    let res = crate::graphql::query::<res::Res>(&q).await?;
    match crate::config::FORMAT.get() {
        Some(&crate::config::Format::Json) => println!("{}", serde_json::to_string_pretty(&res)?),
        _ => print_text(&res)?,
    }
    Ok(())
}

fn print_text(res: &res::Res) -> surf::Result<()> {
    let calendar = &res.data.user.contributions_collection.contribution_calendar;
    for week in &calendar.weeks {
        print!("{}: ", week.first_day);
        for day in &week.contribution_days {
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
