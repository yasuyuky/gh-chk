use crate::config::TOKEN;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub async fn query<T: DeserializeOwned>(q: &serde_json::Value) -> surf::Result<T> {
    let mut res = surf::post(crate::config::github_graphql_url())
        .header("Authorization", format!("bearer {}", *TOKEN))
        .header("Content-Type", "application/json")
        .header("Accept", "application/vnd.github.merge-info-preview+json")
        .body(q.to_string())
        .await?;
    let status = res.status();
    let body = res.body_string().await?;
    parse_response_body(status, &body)
}

fn parse_response_body<T: DeserializeOwned>(
    status: surf::StatusCode,
    body: &str,
) -> surf::Result<T> {
    if let Some(msg) = graphql_error_message(body) {
        let status = if status.is_success() {
            surf::StatusCode::BadRequest
        } else {
            status
        };
        return Err(surf::Error::from_str(
            status,
            format!("GitHub GraphQL error: {}", msg),
        ));
    }

    if !status.is_success() {
        return Err(surf::Error::from_str(
            status,
            format!(
                "GitHub GraphQL request failed ({}): {}",
                status,
                body.trim()
            ),
        ));
    }

    serde_json::from_str::<T>(body).map_err(|err| {
        surf::Error::from_str(
            surf::StatusCode::InternalServerError,
            format!("Failed to parse GitHub GraphQL response: {}", err),
        )
    })
}

fn graphql_error_message(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let messages = value
        .get("errors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|err| err.get("message").and_then(Value::as_str))
        .collect::<Vec<_>>();

    if !messages.is_empty() {
        return Some(messages.join("; "));
    }

    value
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct ViewerRes {
        data: ViewerData,
    }

    #[derive(Debug, Deserialize)]
    struct ViewerData {
        viewer: Viewer,
    }

    #[derive(Debug, Deserialize)]
    struct Viewer {
        login: String,
    }

    #[test]
    fn parses_success_response() {
        let res: ViewerRes = parse_response_body(
            surf::StatusCode::Ok,
            r#"{"data":{"viewer":{"login":"octocat"}}}"#,
        )
        .unwrap();

        assert_eq!(res.data.viewer.login, "octocat");
    }

    #[test]
    fn reports_graphql_errors() {
        let err = parse_response_body::<ViewerRes>(
            surf::StatusCode::Ok,
            r#"{"errors":[{"message":"Bad credentials"},{"message":"Resource not accessible"}]}"#,
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("Bad credentials"));
        assert!(msg.contains("Resource not accessible"));
        assert!(!msg.contains("missing field"));
    }

    #[test]
    fn reports_http_error_message() {
        let err = parse_response_body::<ViewerRes>(
            surf::StatusCode::Unauthorized,
            r#"{"message":"Bad credentials","documentation_url":"https://docs.github.com/graphql"}"#,
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("Bad credentials"));
        assert!(!msg.contains("missing field"));
    }
}
