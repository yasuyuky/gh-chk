use colored::Colorize;
use serde_json::json;

nestruct::nest! {
    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(rename_all="camelCase")]
    Res {
        data: {
            repository_owner: {
                repositories: {
                    nodes: [{
                        name: String,
                        issues: {
                            nodes: [{
                                number: usize,
                                title: String,
                                url: String
                            }]
                        }
                    }]
                }
            }
        }
    }
}

pub async fn check(slugs: Vec<String>) -> surf::Result<()> {
    let slugs = if slugs.is_empty() {
        vec![crate::cmd::viewer::get().await?]
    } else {
        slugs
    };
    for slug in slugs {
        let vs: Vec<String> = slug.split('/').map(String::from).collect();
        match vs.len() {
            1 => check_owner(&vs[0]).await?,
            _ => panic!("unknown slug format"),
        }
    }
    Ok(())
}

async fn check_owner(owner: &str) -> surf::Result<()> {
    let v = json!({ "login": owner });
    let q = json!({ "query": include_str!("../query/issues.graphql"), "variables": v });
    let res = crate::graphql::query::<res::Res>(&q).await?;
    match crate::config::FORMAT.get() {
        Some(&crate::config::Format::Json) => println!("{}", serde_json::to_string_pretty(&res)?),
        _ => print_text(&res),
    }
    Ok(())
}

fn print_text(res: &res::Res) {
    let mut count = 0usize;
    for repo in &res.data.repository_owner.repositories.nodes {
        if repo.issues.nodes.is_empty() {
            continue;
        }
        println!("{}", repo.name.cyan());
        for issue in &repo.issues.nodes {
            count += 1;
            println!("  #{} {} {} ", issue.number, issue.url, issue.title)
        }
    }
    println!("Count of Issues: {count}");
}
