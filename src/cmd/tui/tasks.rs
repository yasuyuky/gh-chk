//! Network work owns its inputs and returns events; it never mutates the UI.
use super::{PrNode, PreviewMode, build_commit_graph_entries, make_commit_graph_text};
use crate::cmd::{contributions, prs, search, viewer};
use crate::{slug::Slug, styling};
use async_std::channel::{Receiver, Sender};
use ratatui::text::Text;

pub(super) fn start(task: Task, specs: Vec<Slug>) -> Receiver<TaskEvent> {
    let (tx, rx) = async_std::channel::bounded(1);
    async_std::task::spawn(async move {
        let event = task.run(specs).await;
        let _ = tx.send(event).await;
    });
    rx
}

pub(super) fn start_merge(pr: &PrNode) -> Receiver<surf::Result<crate::merge::MergeOutcome>> {
    let (tx, rx) = async_std::channel::bounded(1);
    let slug = pr.slug();
    let number = pr.number;
    async_std::task::spawn(async move {
        let result = crate::merge::merge_pr(&slug, number).await;
        let _ = tx.send(result).await;
    });
    rx
}

pub(super) fn start_auto_reload(specs: Vec<Slug>, id: u64, tx: Sender<super::AutoReloadEvent>) {
    async_std::task::spawn(async move {
        let result = prs::fetch_prs_with_warnings(&specs).await;
        let _ = tx.send(super::AutoReloadEvent { id, result }).await;
    });
}

#[derive(Debug, Clone)]
pub(super) enum Task {
    Reload,
    ReloadRepo {
        pr: PrNode,
        message: Option<String>,
    },
    Approve(PrNode),
    Contributions,
    Preview {
        pr: PrNode,
        mode: PreviewMode,
    },
    Search {
        owner: String,
        query: String,
        revision: u64,
    },
}

impl Task {
    pub fn status(&self) -> String {
        match self {
            Self::Reload => "🔄 Reloading all...".to_string(),
            Self::ReloadRepo {
                message: Some(message),
                ..
            } => format!("{message} Reloading..."),
            Self::ReloadRepo { pr, message: None } => format!("🔄 Reloading {}...", pr.numslug()),
            Self::Approve(pr) => format!("Approving PR {}...", pr.numslug()),
            Self::Contributions => "🔄 Reloading contrib...".to_string(),
            Self::Preview { pr, mode } => format!("🔎 Loading {mode} for #{}...", pr.number),
            Self::Search { owner, .. } => format!("🔎 Searching code for user:{owner}..."),
        }
    }

    pub fn changes_prs(&self) -> bool {
        matches!(
            self,
            Self::Reload | Self::ReloadRepo { .. } | Self::Approve(_)
        )
    }

    pub fn same_request(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Reload, Self::Reload) | (Self::Contributions, Self::Contributions) => true,
            (Self::Approve(a), Self::Approve(b)) => a.id == b.id,
            (
                Self::ReloadRepo {
                    pr: a,
                    message: None,
                },
                Self::ReloadRepo {
                    pr: b,
                    message: None,
                },
            ) => a.id == b.id,
            (Self::Preview { pr: a, mode: x }, Self::Preview { pr: b, mode: y }) => {
                a.id == b.id && x == y
            }
            (Self::Search { revision: a, .. }, Self::Search { revision: b, .. }) => a == b,
            _ => false,
        }
    }

    pub async fn run(self, mut specs: Vec<Slug>) -> TaskEvent {
        match self {
            Self::Reload => {
                let result = async {
                    if specs.is_empty() {
                        specs.push(Slug::Owner(viewer::get().await?));
                    }
                    let result = prs::fetch_prs_with_warnings(&specs).await?;
                    Ok((specs, result))
                }
                .await;
                TaskEvent::Reloaded(result)
            }
            Self::ReloadRepo { pr, message } => {
                let result = prs::fetch_repo_prs_with_warnings(
                    &pr.repository.owner.login,
                    &pr.repository.name,
                )
                .await;
                TaskEvent::RepoReloaded {
                    pr,
                    message,
                    result,
                }
            }
            Self::Approve(pr) => {
                let result = prs::approve_pr(&pr.id).await;
                TaskEvent::Approved { pr, result }
            }
            Self::Contributions => {
                let result = async {
                    let profile = viewer::get_profile().await?;
                    let calendar = contributions::fetch_calendar(&profile.login).await?;
                    Ok((profile, calendar))
                }
                .await;
                TaskEvent::ContributionsLoaded(result)
            }
            Self::Preview { pr, mode } => {
                let result = load_preview(&pr, mode).await;
                TaskEvent::PreviewLoaded { pr, mode, result }
            }
            Self::Search {
                owner,
                query,
                revision,
            } => {
                let result = search::search_code(&owner, &query).await;
                TaskEvent::SearchCompleted { revision, result }
            }
        }
    }
}

pub(super) enum TaskEvent {
    Reloaded(surf::Result<(Vec<Slug>, prs::PullRequestFetchResult)>),
    RepoReloaded {
        pr: PrNode,
        message: Option<String>,
        result: surf::Result<prs::PullRequestFetchResult>,
    },
    Approved {
        pr: PrNode,
        result: surf::Result<()>,
    },
    ContributionsLoaded(surf::Result<(viewer::Profile, contributions::model::Res)>),
    PreviewLoaded {
        pr: PrNode,
        mode: PreviewMode,
        result: surf::Result<Text<'static>>,
    },
    SearchCompleted {
        revision: u64,
        result: surf::Result<Vec<search::SearchItem>>,
    },
}

async fn load_preview(pr: &PrNode, mode: PreviewMode) -> surf::Result<Text<'static>> {
    let owner = &pr.repository.owner.login;
    let name = &pr.repository.name;
    match mode {
        PreviewMode::Body => {
            let body = prs::fetch_pr_body(owner, name, pr.number).await?;
            Ok(styling::prettify_pr_preview(&pr.title, &pr.url, &body))
        }
        PreviewMode::Diff => {
            let files = prs::fetch_pr_diffs(owner, name, pr.number).await?;
            let mut out = String::default();
            for file in files {
                out.push_str(&file.to_string());
            }
            if out.is_empty() {
                out.push_str("No file changes found.");
            }
            Ok(styling::make_diff_text(&out))
        }
        PreviewMode::Commits => {
            let commits = prs::fetch_pr_commits(owner, name, pr.number).await?;
            Ok(make_commit_graph_text(&build_commit_graph_entries(
                &commits,
            )))
        }
    }
}
