use serde::de::DeserializeOwned;

const BASE_URI: &str = "https://api.github.com/";

fn parse_next(res: &surf::Response) -> Option<String> {
    let link = match res.header("Link") {
        Some(vs) => vs,
        None => return None,
    };
    for l in link.as_str().split(",") {
        if l.contains("next") {
            return Some(l[(l.find('<').unwrap() + 1)..l.find('>').unwrap()].to_owned());
        }
    }
    return None;
}

pub async fn get<T: DeserializeOwned>(
    path: &str,
    cond: fn(i: usize) -> bool,
) -> surf::Result<Vec<T>> {
    let uri = BASE_URI.to_owned() + path;
    let token = std::env::var("GITHUB_TOKEN")?;
    let mut res = surf::get(&uri)
        .header("Authorization", format!("token {}", token))
        .await?;
    let mut result: Vec<T> = Vec::new();
    let mut i = 0usize;
    if !cond(i) {
        return Ok(result);
    }
    result.append(&mut res.body_json::<Vec<T>>().await?);
    while let Some(link) = parse_next(&res) {
        res = surf::get(&link)
            .header("Authorization", format!("token {}", token))
            .await?;
        i += 1;
        if !cond(i) {
            break;
        };
        result.append(&mut res.body_json::<Vec<T>>().await?);
    }
    Ok(result)
}
