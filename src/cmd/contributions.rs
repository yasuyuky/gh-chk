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
                                date: String
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
    let mut year_to_date = 0;
    let mut month_to_date = 0;
    let this_week = calendar.weeks.last().unwrap();
    let today = this_week.contribution_days.last().unwrap().date.clone();
    let today_year = today.chars().take(4).collect::<String>();
    let today_month = today.chars().take(7).collect::<String>();

    for week in &calendar.weeks {
        print!("{}: ", week.first_day);
        let mut week_count = 0f64;
        for day in &week.contribution_days {
            week_count += day.contribution_count as f64;
            let r = u8::from_str_radix(day.color.get(1..3).unwrap_or_default(), 16)?;
            let g = u8::from_str_radix(day.color.get(3..5).unwrap_or_default(), 16)?;
            let b = u8::from_str_radix(day.color.get(5..7).unwrap_or_default(), 16)?;
            let cnt = format!("{:3}", day.contribution_count);
            print!("{} ", cnt.as_str().color("black").on_truecolor(r, g, b));
            if day.date.starts_with(&today_year) {
                year_to_date += day.contribution_count;
            }
            if day.date.starts_with(&today_month) {
                month_to_date += day.contribution_count;
            }
        }
        let l = week.contribution_days.len() as f64;
        print!(" {:3} {:>5.2}", week_count, week_count / l);
        println!();
    }
    println!("total contributions: {}", calendar.total_contributions);
    println!("year to date: {}", year_to_date);
    println!("month to date: {}", month_to_date);
    Ok(())
}
