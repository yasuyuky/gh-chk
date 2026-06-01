use serde_json::json;

pub struct Profile {
    pub login: String,
    pub url: String,
}

nestruct::nest! {
    #[derive(serde::Deserialize, serde::Serialize)]
    Res {
        data: {
            viewer: {
                login: String,
                url: String
            }
        }
    }
}

pub async fn get_profile() -> surf::Result<Profile> {
    let q = json!({ "query": include_str!("../query/viewer.graphql") });
    let res = crate::graphql::query::<res::Res>(&q).await?;
    Ok(Profile {
        login: res.data.viewer.login,
        url: res.data.viewer.url,
    })
}

pub async fn get() -> surf::Result<String> {
    Ok(get_profile().await?.login)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_profile_url() {
        let res: res::Res = serde_json::from_str(
            r#"{"data":{"viewer":{"login":"octocat","url":"https://example.test/octocat"}}}"#,
        )
        .unwrap();

        assert_eq!(res.data.viewer.login, "octocat");
        assert_eq!(res.data.viewer.url, "https://example.test/octocat");
    }
}
