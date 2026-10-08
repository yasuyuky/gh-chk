use crate::config::{self, TOKEN};
use async_std::{future::timeout, task::sleep};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;
use surf::{StatusCode, http::Method};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const MERGE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeOutcome {
    Merged,
    Enqueued,
}

impl MergeOutcome {
    pub fn message(self, pr: &str) -> String {
        match self {
            Self::Merged => format!("✅ Merged PR {pr}"),
            Self::Enqueued => format!("🕓 Added PR {pr} to the merge queue (not yet merged)"),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", content = "details", rename_all = "snake_case")]
enum MergeResponse {
    Pending { uuid: String },
    Merged {},
    Enqueued {},
    Failed { message: String },
}

pub async fn merge_pr(slug: &str, number: usize) -> surf::Result<MergeOutcome> {
    let path = format!("repos/{slug}/pulls/{number}/merge-async");
    let response = timeout(MERGE_TIMEOUT, request(Method::Put, &path))
        .await
        .map_err(|_| {
            surf::Error::from_str(
                StatusCode::GatewayTimeout,
                "Timed out submitting the merge request; it may still be running on GitHub. Check the PR before retrying.",
            )
        })??;
    match response {
        MergeResponse::Pending { uuid } => wait_for_merge(&path, &uuid, MERGE_TIMEOUT).await,
        response => outcome(response),
    }
}

async fn wait_for_merge(path: &str, uuid: &str, wait: Duration) -> surf::Result<MergeOutcome> {
    if uuid.is_empty() || !uuid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err(surf::Error::from_str(
            StatusCode::BadGateway,
            "GitHub returned an invalid merge request ID",
        ));
    }
    let path = format!("{path}/{uuid}");
    timeout(wait, async {
        loop {
            sleep(POLL_INTERVAL).await;
            let response = request(Method::Get, &path).await.map_err(|err| {
                surf::Error::from_str(
                    err.status(),
                    format!("Could not check merge request {uuid}: {err}. It may still be running on GitHub; check the PR before retrying."),
                )
            })?;
            if !matches!(response, MergeResponse::Pending { .. }) {
                return outcome(response);
            }
        }
    })
    .await
    .map_err(|_| {
        surf::Error::from_str(
            StatusCode::GatewayTimeout,
            format!("Timed out waiting for merge request {uuid}; it may still be running on GitHub. Check the PR or GET {path} before retrying."),
        )
    })?
}

fn outcome(response: MergeResponse) -> surf::Result<MergeOutcome> {
    match response {
        MergeResponse::Merged {} => Ok(MergeOutcome::Merged),
        MergeResponse::Enqueued {} => Ok(MergeOutcome::Enqueued),
        MergeResponse::Failed { message } => Err(surf::Error::from_str(
            StatusCode::BadRequest,
            format!("GitHub merge failed: {message}"),
        )),
        MergeResponse::Pending { .. } => Err(surf::Error::from_str(
            StatusCode::BadGateway,
            "GitHub merge is still pending",
        )),
    }
}

async fn request(method: Method, path: &str) -> surf::Result<MergeResponse> {
    if TOKEN.is_empty() {
        return Err(surf::Error::from_str(
            StatusCode::Unauthorized,
            "No GitHub token found. Run `gh auth login -h github.com` or set GITHUB_TOKEN.",
        ));
    }
    let url = surf::Url::parse(&config::github_api_url(path))?;
    let mut request = surf::RequestBuilder::new(method, url)
        .header("Authorization", format!("bearer {}", *TOKEN))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2026-03-10");
    if let Ok(timezone) = iana_time_zone::get_timezone() {
        request = request.header("Time-Zone", timezone);
    }
    if method == Method::Put {
        request = request.body_json(&json!({
            "merge_action": "default",
            "bypass_rules": false,
        }))?;
    }
    let mut response = request.await?;
    let status = response.status();
    let body = response.body_string().await?;
    parse_response(method, status, &body)
}

fn parse_response(method: Method, status: StatusCode, body: &str) -> surf::Result<MergeResponse> {
    let response = serde_json::from_str::<MergeResponse>(body);
    // A conflicting submission returns the existing request, which we can follow.
    if status.is_success()
        || (method == Method::Put
            && status == StatusCode::Conflict
            && matches!(&response, Ok(MergeResponse::Pending { .. })))
    {
        return response.map_err(|err| {
            surf::Error::from_str(
                StatusCode::BadGateway,
                format!("Invalid GitHub async merge response: {err}"),
            )
        });
    }
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/details/message")
                .or_else(|| value.get("message"))
                .and_then(|value| value.as_str().map(str::to_owned))
        })
        .unwrap_or_else(|| body.trim().to_owned());
    Err(surf::Error::from_str(
        status,
        format!("GitHub async merge request failed ({status}): {message}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[async_std::test]
    async fn timeout_preserves_the_request_id_and_does_not_claim_failure() {
        let err = wait_for_merge(
            "repos/owner/repo/pulls/1/merge-async",
            "request-id",
            Duration::ZERO,
        )
        .await
        .unwrap_err();
        assert_eq!(err.status(), StatusCode::GatewayTimeout);
        assert!(err.to_string().contains("merge-async/request-id"));
        assert!(err.to_string().contains("may still be running"));
    }
}
