use serde_json::json;

nestruct::nest! {
    #[derive(serde::Deserialize, serde::Serialize)]
    Res {
        data: {
            viewer: {
                login: String
            }
        }
    }
}

pub async fn get() -> surf::Result<String> {
    let q = json!({ "query": include_str!("../query/viewer.graphql") });
    let res = crate::graphql::query::<res::Res>(&q).await?;
    Ok(res.data.viewer.login)
}
