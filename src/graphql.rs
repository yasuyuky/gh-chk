use crate::config::TOKEN;
use crate::env_keys::ENV_GH_CHK_MOCK_FILE;
use serde::de::DeserializeOwned;

const URI: &str = "https://api.github.com/graphql";

pub async fn query<T: DeserializeOwned>(q: &serde_json::Value) -> surf::Result<T> {
    if let Ok(path) = std::env::var(ENV_GH_CHK_MOCK_FILE) {
        let data = std::fs::read_to_string(path)?;
        let res = serde_json::from_str(&data)?;
        return Ok(res);
    }

    let mut res = surf::post(URI)
        .header("Authorization", format!("bearer {}", *TOKEN))
        .header("Accept", "application/vnd.github.merge-info-preview+json")
        .body(q.to_string())
        .await?;
    res.body_json::<T>().await
}
