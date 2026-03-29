#[derive(Debug, Clone)]
pub enum Slug {
    Owner(String),
    Repo { owner: String, name: String },
}

impl TryFrom<&str> for Slug {
    type Error = surf::Error;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        let parts: Vec<&str> = s.split('/').collect();
        match parts.as_slice() {
            [owner] if !owner.is_empty() => Ok(Slug::Owner((*owner).to_string())),
            [owner, name] if !owner.is_empty() && !name.is_empty() => Ok(Slug::Repo {
                owner: (*owner).to_string(),
                name: (*name).to_string(),
            }),
            _ => Err(surf::Error::from_str(
                surf::StatusCode::BadRequest,
                format!("invalid slug spec: {s}"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Slug;

    #[test]
    fn parse_owner_slug() {
        let slug = Slug::try_from("owner").expect("parse owner slug");
        assert!(matches!(slug, Slug::Owner(owner) if owner == "owner"));
    }

    #[test]
    fn parse_repo_slug() {
        let slug = Slug::try_from("owner/repo").expect("parse repo slug");
        assert!(matches!(
            slug,
            Slug::Repo { owner, name } if owner == "owner" && name == "repo"
        ));
    }

    #[test]
    fn reject_invalid_slug() {
        assert!(Slug::try_from("").is_err());
        assert!(Slug::try_from("owner/repo/extra").is_err());
    }
}
