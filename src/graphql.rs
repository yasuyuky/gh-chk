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

pub trait PaginatedGraphQLResponse {
    type Item;

    fn split_page(self) -> (Vec<Self::Item>, bool, Option<String>);
}

pub async fn query_all_pages<T>(
    mut build_query: impl FnMut(Option<&str>) -> serde_json::Value,
) -> surf::Result<Vec<T::Item>>
where
    T: DeserializeOwned + PaginatedGraphQLResponse,
{
    let mut after: Option<String> = None;
    let mut items: Vec<T::Item> = Vec::new();

    loop {
        let q = build_query(after.as_deref());
        let res = query::<T>(&q).await?;
        let (mut page_items, has_next_page, next_after) = res.split_page();
        items.append(&mut page_items);

        if !has_next_page {
            break;
        }

        after = next_after;
        if after.is_none() {
            return Err(surf::Error::from_str(
                surf::StatusCode::InternalServerError,
                "Inconsistent GraphQL pagination: has_next_page is true but next_after is None",
            ));
        }
    }

    Ok(items)
}
