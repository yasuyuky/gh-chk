#[derive(Debug, Clone)]
pub enum SlugSpec {
    Owner(String),
    Repo { owner: String, name: String },
}

impl From<&str> for SlugSpec {
    fn from(s: &str) -> Self {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() == 1 {
            SlugSpec::Owner(parts[0].to_string())
        } else if parts.len() == 2 {
            SlugSpec::Repo {
                owner: parts[0].to_string(),
                name: parts[1].to_string(),
            }
        } else {
            panic!("Invalid slug spec: {}", s);
        }
    }
}
