use crate::config::TOKEN;
use colored::Colorize;

nestruct::nest! {
    #[derive(serde::Serialize, serde::Deserialize)]
    Search {
        total_count: usize,
        incomplete_results: bool,
        items: [{
            name: String,
            path: String,
            sha: String,
            url: String,
            html_url: String,
            score: f64,
            repository: {
                full_name: String,
                html_url: String,
            }
        }]
    }

}

#[derive(Debug, clap::Parser, serde::Serialize)]
pub struct Query {
    q: String,
    /// Search by user
    #[clap(long, short, alias = "owner")]
    user: Option<String>,
}

impl Query {
    fn to_api(&self) -> ApiQuery {
        let q = self.q.to_owned()
            + match &self.user {
                Some(user) => format!(" user:{}", user),
                None => "".to_owned(),
            }
            .as_str();
        ApiQuery {
            q,
            page: 0,
            per_page: 100,
        }
    }
}

#[derive(Debug, clap::Parser, serde::Serialize)]
struct ApiQuery {
    q: String,
    page: usize,
    per_page: u8,
}

pub async fn search(q: &Query) -> surf::Result<()> {
    let mut res = surf::get("https://api.github.com/search/code")
        .header("Authorization", format!("token {}", *TOKEN))
        .query(&q.to_api())?
        .await?;
    let search_result = res.body_json::<search::Search>().await?;
    match crate::config::FORMAT.get() {
        Some(&crate::config::Format::Json) => {
            println!("{}", serde_json::to_string_pretty(&search_result)?)
        }
        _ => print_text(&search_result),
    }
    Ok(())
}

fn print_text(res: &search::Search) {
    for n in &res.items {
        println!(
            "{} {} {}",
            n.repository.full_name.cyan(),
            n.path.yellow(),
            n.html_url
        )
    }
    println!("# count: {}", res.items.len());
}
