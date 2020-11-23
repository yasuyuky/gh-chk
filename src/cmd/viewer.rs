use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Res {
    data: Data,
}
#[derive(Deserialize)]
struct Data {
    viewer: Viewer,
}
#[derive(Deserialize)]
struct Viewer {
    login: String,
}

pub async fn get() -> surf::Result<String> {
    let q = json!({ "query": include_str!("../query/viewer.graphql") });
    let res = crate::graphql::query::<Res>(&q).await?;
    Ok(res.data.viewer.login)
}
