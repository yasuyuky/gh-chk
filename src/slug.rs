#[derive(Debug, Clone)]
pub enum Slug {
    Owner(String),
    Repo { owner: String, name: String },
}

impl From<&str> for Slug {
    fn from(s: &str) -> Self {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() == 1 {
            Slug::Owner(parts[0].to_string())
        } else if parts.len() == 2 {
            Slug::Repo {
                owner: parts[0].to_string(),
                name: parts[1].to_string(),
            }
        } else {
            panic!("Invalid slug spec: {}", s);
        }
    }
}
