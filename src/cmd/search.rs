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
            repository: {
                full_name: String,
                html_url: String,
            }
        }]
    }

}

#[derive(serde::Serialize)]
struct Query {
    q: String,
    page: usize,
    per_page: u8,
}

pub async fn search(query: &str) -> surf::Result<()> {
    let q = Query {
        q: query.to_owned(),
        page: 1,
        per_page: 100,
    };
    let mut res = surf::get("https://api.github.com/search/code")
        .header("Authorization", format!("token {}", *TOKEN))
        .query(&q)?
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
        println!("{} {}", n.repository.full_name.cyan(), n.path.yellow(),)
    }
    println!("# count: {}", res.items.len());
}
