use crate::config::TOKEN;
use crate::env_keys::ENV_GH_CHK_MOCK_FILE;
use serde::de::DeserializeOwned;

const URI: &str = "https://api.github.com/graphql";

fn read_mock_file() -> surf::Result<Option<String>> {
    match std::env::var(ENV_GH_CHK_MOCK_FILE) {
        Ok(path) => Ok(Some(std::fs::read_to_string(path)?)),
        Err(_) => Ok(None),
    }
}

pub async fn query<T: DeserializeOwned>(q: &serde_json::Value) -> surf::Result<T> {
    if let Some(data) = read_mock_file()? {
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

fn read_mock_pages<T>(data: &str) -> surf::Result<Vec<T::Item>>
where
    T: DeserializeOwned + PaginatedGraphQLResponse,
{
    if let Ok(pages) = serde_json::from_str::<Vec<T>>(data) {
        let mut items = Vec::new();
        for page in pages {
            items.extend(page.split_page().0);
        }
        return Ok(items);
    }

    let page = serde_json::from_str::<T>(data)?;
    let (items, has_next_page, _) = page.split_page();
    if has_next_page {
        return Err(surf::Error::from_str(
            surf::StatusCode::InternalServerError,
            "Mock pagination needs a JSON array with one response per page",
        ));
    }
    Ok(items)
}

pub async fn query_all_pages<T>(
    mut build_query: impl FnMut(Option<&str>) -> serde_json::Value,
) -> surf::Result<Vec<T::Item>>
where
    T: DeserializeOwned + PaginatedGraphQLResponse,
{
    if let Some(data) = read_mock_file()? {
        return read_mock_pages::<T>(&data);
    }

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
