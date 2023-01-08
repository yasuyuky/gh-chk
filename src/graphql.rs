use crate::config::TOKEN;
use serde::de::DeserializeOwned;

const URI: &str = "https://api.github.com/graphql";

pub async fn query<T: DeserializeOwned>(q: &serde_json::Value) -> surf::Result<T> {
    let mut res = surf::post(URI)
        .header("Authorization", format!("bearer {}", *TOKEN))
        .header("Accept", "application/vnd.github.merge-info-preview+json")
        .body(q.to_string())
        .await?;
    res.body_json::<T>().await
}
