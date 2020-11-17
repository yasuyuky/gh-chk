use serde::de::DeserializeOwned;

const URI: &str = "https://api.github.com/graphql";

pub async fn query<T: DeserializeOwned>(q: &serde_json::Value) -> surf::Result<T> {
    let token = std::env::var("GITHUB_TOKEN")?;
    let mut res = surf::post(&URI)
        .header("Authorization", format!("bearer {}", token))
        .body(q.to_string())
        .await?;
    Ok(res.body_json::<T>().await?)
}
