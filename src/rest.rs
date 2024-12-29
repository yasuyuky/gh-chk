use crate::config::TOKEN;
use serde::de::DeserializeOwned;
use surf::http::convert::Serialize;

const BASE_URI: &str = "https://api.github.com/";

#[allow(dead_code)]
fn parse_next(res: &surf::Response) -> Option<String> {
    let link = match res.header("Link") {
        Some(vs) => vs,
        None => return None,
    };
    for l in link.as_str().split(',') {
        if l.contains("next") {
            return Some(l[(l.find('<').unwrap() + 1)..l.find('>').unwrap()].to_owned());
        }
    }
    None
}

pub async fn get<T: DeserializeOwned>(path: &str, page: usize) -> surf::Result<Vec<T>> {
    let uri = BASE_URI.to_owned() + path;
    let mut res = get_page(&uri, page).await?;
    res.body_json().await
}

#[derive(Serialize)]
struct Query {
    page: usize,
    per_page: u8,
}

pub async fn get_page(url: &str, page: usize) -> surf::Result<surf::Response> {
    let mut query = HashMap::new();
    query.insert("page", page.to_string());
    query.insert("per_page", 100.to_string());
    surf::get(url)
        .header("Authorization", format!("token {}", *TOKEN))
        .query(&query)?
        .await
}

pub async fn patch(path: &str) -> surf::Result<surf::Response> {
    let uri = BASE_URI.to_owned() + path;
    surf::patch(uri)
        .header("Authorization", format!("token {}", *TOKEN))
        .await
}
