mod commit_graph;
mod contributions;
mod tasks;
mod terminal;
mod view;

use crate::cmd::prs::{self, MergeStateStatus};
use crate::cmd::search::SearchItem;
use crate::merge::MergeOutcome;
use crate::slug::Slug;
use async_std::channel::{Receiver, Sender, TryRecvError};
use contributions::ContributionView;
use crossterm::event::{self, Event, KeyCode, MouseEventKind};
use open;
use ratatui::{Terminal, backend::CrosstermBackend, style::Color, text::Text, widgets::ListState};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tasks::{Task, TaskEvent};
#[cfg(test)]
use view::{
    build_preview_text, build_search_input_help, pr_list_line, prs_help_base, render_pr_list,
    search_help_base,
};

// Type alias for GraphQL PR node for brevity (reuse prs module types)
type PrNode = prs::pull_request::PullRequest;
const SEARCH_HISTORY_LIMIT: usize = 100;
pub const AUTO_RELOAD_DEFAULT_SECS: u64 = 300;
pub const AUTO_RELOAD_MIN_SECS: u64 = 60;

impl MergeStateStatus {
    fn to_color(&self) -> Color {
        match self {
            Self::Behind => Color::Yellow,
            Self::Blocked => Color::Red,
            Self::Clean => Color::Green,
            Self::Dirty => Color::Yellow,
            Self::Draft => Color::White,
            Self::HasHooks => Color::Yellow,
            Self::Unknown => Color::Magenta,
            Self::Unstable => Color::Yellow,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Preview {
    mode: Option<PreviewMode>,
    scroll: u16,
    height: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PreviewMode {
    Body,
    Diff,
    Commits,
}

impl std::fmt::Display for PreviewMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let as_str = match self {
            Self::Body => "Body",
            Self::Diff => "Diff",
            Self::Commits => "Commits",
        };
        f.write_str(as_str)
    }
}

struct RunningTask {
    task: Task,
    rx: Receiver<TaskEvent>,
    pr_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Prs,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchFocus {
    Input,
    Results,
}

type AutoReloadSender = Sender<AutoReloadEvent>;
type AutoReloadReceiver = Receiver<AutoReloadEvent>;

struct AutoReloadEvent {
    id: u64,
    result: surf::Result<prs::PullRequestFetchResult>,
}

struct MergeTask {
    pr: PrNode,
    rx: Receiver<surf::Result<MergeOutcome>>,
}

struct AutoReload {
    interval: Duration,
    next_at: Instant,
    in_flight: bool,
    next_id: u64,
    ignore_before_id: u64,
    tx: AutoReloadSender,
    rx: AutoReloadReceiver,
}

impl AutoReload {
    fn new(interval: Duration) -> Self {
        let (tx, rx) = async_std::channel::unbounded();
        Self {
            interval,
            next_at: Instant::now() + interval,
            in_flight: false,
            next_id: 1,
            ignore_before_id: 0,
            tx,
            rx,
        }
    }
}

fn mark_auto_reload_complete(auto_reload: &mut AutoReload) {
    auto_reload.in_flight = false;
    auto_reload.next_at = Instant::now() + auto_reload.interval;
}

struct SearchState {
    owner: String,
    query: String,
    history: Vec<String>,
    history_index: Option<usize>,
    history_draft: String,
    results: Vec<SearchItem>,
    list_state: ListState,
    focus: SearchFocus,
    preview_open: bool,
    revision: u64,
}

impl SearchState {
    fn new(owner: String, history: Vec<String>) -> Self {
        Self {
            owner,
            query: String::default(),
            history,
            history_index: None,
            history_draft: String::default(),
            results: Vec::new(),
            list_state: ListState::default(),
            focus: SearchFocus::Input,
            preview_open: false,
            revision: 0,
        }
    }

    fn navigate(&mut self, d: isize) {
        if self.results.is_empty() {
            return;
        }
        let len = self.results.len() as isize;
        let i = (self.list_state.selected().unwrap_or(0) as isize + d).rem_euclid(len);
        self.list_state.select(Some(i as usize));
    }

    fn select_first(&mut self) {
        if self.results.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    fn remember_query(&mut self, query: &str) -> bool {
        let changed = push_search_history(&mut self.history, query);
        self.history_index = None;
        self.history_draft.clear();
        changed
    }

    fn reset_history_nav(&mut self) {
        self.history_index = None;
        self.history_draft.clear();
    }

    fn navigate_history(&mut self, d: isize) {
        if self.history.is_empty() {
            return;
        }
        let next = match (self.history_index, d) {
            (None, -1) => {
                self.history_draft = std::mem::take(&mut self.query);
                Some(self.history.len() - 1)
            }
            (None, 1) => None,
            (Some(0), -1) => Some(0),
            (Some(i), -1) => Some(i - 1),
            (Some(i), 1) if i + 1 < self.history.len() => Some(i + 1),
            (Some(_), 1) => {
                self.history_index = None;
                self.query = std::mem::take(&mut self.history_draft);
                return;
            }
            (current, _) => current,
        };
        let Some(next) = next else {
            return;
        };
        self.history_index = Some(next);
        self.query.clone_from(&self.history[next]);
    }
}

fn search_history_path() -> PathBuf {
    let mut path = crate::config::CONFIG_PATH.clone();
    path.set_file_name("search_history");
    path
}

fn load_search_history(path: &Path) -> Vec<String> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut history = Vec::new();
    for line in content.lines() {
        push_search_history(&mut history, line);
    }
    history
}

fn save_search_history(path: &Path, history: &[String]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut content = history.join("\n");
    if !content.is_empty() {
        content.push('\n');
    }
    fs::write(path, content)
}

fn push_search_history(history: &mut Vec<String>, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return false;
    }
    let before = history.clone();
    history.retain(|item| item != query);
    history.push(query.to_string());
    let excess = history.len().saturating_sub(SEARCH_HISTORY_LIMIT);
    if excess > 0 {
        history.drain(0..excess);
    }
    *history != before
}

fn is_search_back_key(focus: SearchFocus, code: KeyCode) -> bool {
    matches!(
        (focus, code),
        (SearchFocus::Input, KeyCode::Esc) | (SearchFocus::Results, KeyCode::Char('q'))
    )
}

#[cfg(target_os = "macos")]
const CLIPBOARD_COMMANDS: &[(&str, &[&str])] = &[("pbcopy", &[])];

#[cfg(target_os = "windows")]
const CLIPBOARD_COMMANDS: &[(&str, &[&str])] = &[("clip", &[])];

#[cfg(all(unix, not(target_os = "macos")))]
const CLIPBOARD_COMMANDS: &[(&str, &[&str])] = &[
    ("wl-copy", &[]),
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
];

fn copy_to_clipboard(text: &str) -> io::Result<()> {
    let mut last_error = None;
    for (program, args) in CLIPBOARD_COMMANDS {
        match run_clipboard_command(program, args, text) {
            Ok(()) => return Ok(()),
            Err(err) => last_error = Some(err),
        }
    }
    Err(last_error
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "clipboard unavailable")))
}

fn run_clipboard_command(program: &str, args: &[&str], text: &str) -> io::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        // Many clipboard tools read until EOF, so close stdin before wait().
        stdin.write_all(text.as_bytes())?;
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "clipboard command failed: {program}"
        )))
    }
}

struct App {
    prs: Vec<PrNode>,
    pr_revision: u64,
    list_state: ListState,
    should_quit: bool,
    status_message: Option<String>,
    status_clear_at: Option<Instant>,
    specs: Vec<Slug>,
    cache: HashMap<(PreviewMode, String), Text<'static>>, // (mode, pr_id) -> content
    preview: Preview,
    contributions: ContributionView,
    viewer_profile_url: Option<String>,
    pending_tasks: VecDeque<Task>,
    active_task: Option<RunningTask>,
    merge_task: Option<MergeTask>,
    auto_reload: Option<AutoReload>,
    mode: AppMode,
    search: SearchState,
}

impl App {
    fn new(specs: Vec<Slug>, auto_reload: Option<Duration>) -> App {
        let search_owner = Self::default_search_owner(&specs);
        let search_history = load_search_history(&search_history_path());
        App {
            prs: Vec::new(),
            pr_revision: 0,
            list_state: ListState::default(),
            should_quit: false,
            status_message: Some("🔄 Loading PRs...".to_string()),
            status_clear_at: None,
            specs,
            cache: HashMap::new(),
            preview: Preview::default(),
            contributions: ContributionView::default(),
            viewer_profile_url: None,
            pending_tasks: VecDeque::from([Task::Reload, Task::Contributions]),
            active_task: None,
            merge_task: None,
            auto_reload: auto_reload.map(AutoReload::new),
            mode: AppMode::Prs,
            search: SearchState::new(search_owner, search_history),
        }
    }

    fn default_search_owner(specs: &[Slug]) -> String {
        specs
            .first()
            .map(|slug| match slug {
                Slug::Owner(owner) => owner.clone(),
                Slug::Repo { owner, .. } => owner.clone(),
            })
            .unwrap_or_default()
    }

    fn navigate(&mut self, d: isize) {
        if self.prs.is_empty() {
            return;
        }
        let i = (self.list_state.selected().unwrap_or(0) as isize + d) % self.prs.len() as isize;
        self.list_state.select(Some(i as usize));
        self.preview.scroll = 0;
    }

    fn get_selected_pr(&self) -> Option<&PrNode> {
        self.list_state.selected().and_then(|i| self.prs.get(i))
    }

    fn merge_selected(&mut self) {
        if self.merge_task.is_some() {
            return;
        }
        let Some(pr) = self.get_selected_pr().cloned() else {
            return;
        };
        self.ignore_auto_reload_result();
        self.set_status_persistent(format!("Merging PR {}...", pr.numslug()));
        let rx = tasks::start_merge(&pr);
        self.merge_task = Some(MergeTask { pr, rx });
    }

    fn finish_merge(&mut self) {
        let Some(task) = &self.merge_task else {
            return;
        };
        let result = match task.rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Closed) => Err(surf::Error::from_str(
                surf::StatusCode::InternalServerError,
                "Merge result unavailable; check the PR on GitHub before retrying",
            )),
        };
        let pr = task.pr.clone();
        self.merge_task = None;
        self.ignore_auto_reload_result();
        self.defer_auto_reload();
        match result {
            Ok(outcome) => {
                self.pr_revision += 1;
                let message = outcome.message(&pr.numslug());
                self.set_status(message.clone());
                self.queue_task(Task::ReloadRepo {
                    pr,
                    message: Some(message),
                });
                if outcome == MergeOutcome::Merged
                    && !self
                        .pending_tasks
                        .iter()
                        .any(|task| matches!(task, Task::Contributions))
                {
                    // An active request may contain a calendar fetched before this merge.
                    self.pending_tasks.push_back(Task::Contributions);
                }
            }
            Err(err) => self.set_status(format!("❌ Merge request for PR {}: {err}", pr.numslug())),
        }
    }

    fn open_url(&self) {
        if let Some(pr) = self.get_selected_pr()
            && let Err(e) = open::that(&pr.url)
        {
            eprintln!("Failed to open URL: {}", e);
        }
    }

    fn scroll_preview_down(&mut self, n: u16) {
        if self.preview.mode.is_some() {
            self.preview.scroll = self.preview.scroll.saturating_add(n);
        }
    }
    fn scroll_preview_up(&mut self, n: u16) {
        if self.preview.mode.is_some() {
            self.preview.scroll = self.preview.scroll.saturating_sub(n);
        }
    }

    fn apply_pr_list_and_restore_selection(&mut self, new_list: Vec<PrNode>) {
        self.pr_revision += 1;
        let sel = self.list_state.selected().unwrap_or(0);
        let selected_id = self.get_selected_pr().map(|pr| pr.id.clone());
        self.prs = new_list;
        if self.prs.is_empty() {
            self.list_state.select(None);
        } else {
            let selection = selected_id
                .and_then(|id| self.prs.iter().position(|pr| pr.id == id))
                .unwrap_or_else(|| sel.min(self.prs.len().saturating_sub(1)));
            self.list_state.select(Some(selection));
        }
        self.prune_cache_to_existing();
    }

    fn start_auto_reload(&mut self) {
        if self.mode != AppMode::Prs
            || !self.pending_tasks.is_empty()
            || self.active_task.is_some()
            || self.merge_task.is_some()
        {
            return;
        }
        let Some(auto_reload) = &mut self.auto_reload else {
            return;
        };
        let now = Instant::now();
        if auto_reload.in_flight || now < auto_reload.next_at {
            return;
        }

        let id = auto_reload.next_id;
        auto_reload.next_id += 1;
        auto_reload.in_flight = true;

        tasks::start_auto_reload(self.specs.clone(), id, auto_reload.tx.clone());
        self.set_status_persistent("🔄 Auto reloading PRs...");
    }

    fn finish_auto_reload(&mut self) {
        let Some(event) = self.take_auto_reload_event() else {
            return;
        };

        if self.is_stale_auto_reload(event.id) {
            return;
        }

        let show_status = self.mode == AppMode::Prs;
        match event.result {
            Ok(result) => {
                let count = result.prs.len();
                let warning = result.warning;
                self.apply_pr_list_and_restore_selection(result.prs);
                let queued_preview = show_status && self.queue_preview_if_needed();
                if show_status && let Some(warning) = warning {
                    self.set_status(format!(
                        "⚠️ Auto reloaded. {} PRs. {}",
                        count,
                        warning.message()
                    ));
                } else if show_status && !queued_preview {
                    self.set_status(format!("✅ Auto reloaded. {} PRs.", count));
                }
            }
            Err(e) => {
                if show_status {
                    self.set_status(format!("❌ Auto reload error: {}", e));
                }
            }
        }
    }

    fn take_auto_reload_event(&mut self) -> Option<AutoReloadEvent> {
        let auto_reload = self.auto_reload.as_mut()?;
        match auto_reload.rx.try_recv() {
            Ok(event) => {
                mark_auto_reload_complete(auto_reload);
                Some(event)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Closed) => {
                mark_auto_reload_complete(auto_reload);
                None
            }
        }
    }

    fn is_stale_auto_reload(&self, id: u64) -> bool {
        self.auto_reload
            .as_ref()
            .is_some_and(|auto_reload| id < auto_reload.ignore_before_id)
    }

    fn defer_auto_reload(&mut self) {
        let Some(auto_reload) = &mut self.auto_reload else {
            return;
        };
        auto_reload.next_at = Instant::now() + auto_reload.interval;
    }

    fn ignore_auto_reload_result(&mut self) {
        let Some(auto_reload) = &mut self.auto_reload else {
            return;
        };
        if auto_reload.in_flight {
            auto_reload.ignore_before_id = auto_reload.next_id;
        }
    }

    fn queue_preview_if_needed(&mut self) -> bool {
        self.preview
            .mode
            .is_some_and(|mode| self.queue_mode_if_needed(mode))
    }

    fn refresh_preview(&mut self) {
        if let Some(id) = self.get_selected_pr().map(|pr| pr.id.clone()) {
            self.drop_preview_cache_for(&id);
        }
        self.queue_preview_if_needed();
    }

    fn prune_cache_to_existing(&mut self) {
        let ids: HashSet<String> = self.prs.iter().map(|pr| pr.id.clone()).collect();
        self.cache.retain(|(_, pr_id), _| ids.contains(pr_id));
        self.pending_tasks.retain_mut(|task| {
            let Task::Preview { pr, .. } = task else {
                return true;
            };
            let Some(current) = self.prs.iter().find(|current| current.id == pr.id) else {
                return false;
            };
            // Queued previews may outlive the PR details captured by the key press.
            pr.clone_from(current);
            true
        });
    }

    fn drop_preview_cache_for(&mut self, pr_id: &str) {
        self.pending_tasks
            .retain(|task| !matches!(task, Task::Preview { pr, .. } if pr.id == pr_id));
        let id = pr_id.to_string();
        for mode in [PreviewMode::Body, PreviewMode::Diff, PreviewMode::Commits] {
            self.cache.remove(&(mode, id.clone()));
        }
    }

    fn replace_repo_prs(
        &mut self,
        owner: &str,
        name: &str,
        repo_prs: &mut Vec<PrNode>,
        keep_selection: Option<String>,
    ) {
        self.pr_revision += 1;
        let insert_at = self
            .prs
            .iter()
            .position(|pr| pr.repository.owner.login == owner && pr.repository.name == name)
            .unwrap_or(self.prs.len()); // skipcq: RS-W1031

        self.prs
            .retain(|pr| !(pr.repository.owner.login == owner && pr.repository.name == name));

        let mut incoming: Vec<PrNode> = Vec::new();
        incoming.append(repo_prs);
        self.prs.splice(insert_at..insert_at, incoming);

        self.prune_cache_to_existing();

        let selection = keep_selection
            .and_then(|id| self.prs.iter().position(|pr| pr.id == id))
            .or_else(|| {
                if self.prs.is_empty() {
                    None
                } else if insert_at >= self.prs.len() {
                    Some(self.prs.len() - 1)
                } else {
                    Some(insert_at)
                }
            });
        self.list_state.select(selection);
        self.preview.scroll = 0;
    }
}

impl App {
    fn queue_task(&mut self, task: Task) -> bool {
        if self
            .active_task
            .as_ref()
            .is_some_and(|active| active.task.same_request(&task))
            || self
                .pending_tasks
                .iter()
                .any(|pending| pending.same_request(&task))
        {
            return false;
        }
        if task.changes_prs() {
            self.ignore_auto_reload_result();
        }
        if matches!(task, Task::Search { .. }) {
            self.pending_tasks
                .retain(|pending| !matches!(pending, Task::Search { .. }));
        }
        self.pending_tasks.push_back(task);
        true
    }

    fn start_pending_task(&mut self) {
        if self.active_task.is_some() || self.should_quit {
            return;
        }
        while let Some(task) = self.pending_tasks.pop_front() {
            match &task {
                Task::Search { revision, .. } if *revision != self.search.revision => continue,
                Task::Preview { pr, mode }
                    if !self.prs.iter().any(|p| p.id == pr.id)
                        || self.cache.contains_key(&(*mode, pr.id.clone())) =>
                {
                    continue;
                }
                _ => {}
            }
            // Let a completed operation's message remain visible during follow-up reads.
            if self.status_clear_at.is_none() {
                self.set_status_persistent(task.status());
            }
            let rx = tasks::start(task.clone(), self.specs.clone());
            self.active_task = Some(RunningTask {
                task,
                rx,
                pr_revision: self.pr_revision,
            });
            break;
        }
    }

    fn finish_pending_task(&mut self) {
        let Some(active) = &self.active_task else {
            return;
        };
        let result = match active.rx.try_recv() {
            Err(TryRecvError::Empty) => return,
            result => result,
        };
        let active = self
            .active_task
            .take()
            .expect("active task was checked above");
        if self.status_message.as_deref() == Some(active.task.status().as_str()) {
            self.on_clear_help();
        }
        self.defer_auto_reload();
        if active.task.changes_prs() {
            self.ignore_auto_reload_result();
        }
        match result {
            Ok(event) => {
                // A merge or automatic reload may have replaced the data while a read ran.
                if active.pr_revision != self.pr_revision
                    && matches!(
                        event,
                        TaskEvent::Reloaded(_)
                            | TaskEvent::RepoReloaded { .. }
                            | TaskEvent::PreviewLoaded { .. }
                    )
                {
                    if matches!(active.task, Task::Preview { .. }) {
                        self.queue_preview_if_needed();
                    } else {
                        // A merge's follow-up only reloads its own repository.
                        self.queue_task(active.task);
                    }
                    return;
                }
                self.apply_task_event(event);
            }
            Err(_) => {
                let mut message =
                    "❌ Background task stopped before returning a result.".to_string();
                if matches!(active.task, Task::Approve(_)) {
                    message.push_str(" Check GitHub before retrying an approval.");
                }
                self.set_status(message);
            }
        }
    }

    fn apply_task_event(&mut self, event: TaskEvent) {
        match event {
            TaskEvent::Reloaded(result) => self.apply_reload_result(result),
            TaskEvent::RepoReloaded {
                pr,
                message,
                result,
            } => self.apply_repo_reload_result(pr, message, result),
            TaskEvent::Approved { pr, result } => self.apply_approval_result(pr, result),
            TaskEvent::ContributionsLoaded(result) => self.apply_contributions_result(result),
            TaskEvent::PreviewLoaded { pr, mode, result } => {
                self.apply_preview_result(pr, mode, result)
            }
            TaskEvent::SearchCompleted { revision, result } => {
                self.apply_search_result(revision, result)
            }
        }
    }

    fn apply_reload_result(
        &mut self,
        result: surf::Result<(Vec<Slug>, prs::PullRequestFetchResult)>,
    ) {
        match result {
            Ok((specs, result)) => {
                self.specs = specs;
                if self.search.owner.is_empty() {
                    self.search.owner = Self::default_search_owner(&self.specs);
                    for task in &mut self.pending_tasks {
                        if let Task::Search { owner, .. } = task {
                            owner.clone_from(&self.search.owner);
                        }
                    }
                }
                let warning = result.warning;
                self.apply_pr_list_and_restore_selection(result.prs);
                self.refresh_preview();
                if self.mode == AppMode::Prs {
                    let message = format!("Reloaded. {} PRs.", self.prs.len());
                    self.set_reload_status(message, warning);
                }
                self.queue_task(Task::Contributions);
            }
            Err(err) => self.set_status(format!("❌ Reload error: {err}")),
        }
    }

    fn apply_repo_reload_result(
        &mut self,
        pr: PrNode,
        message: Option<String>,
        result: surf::Result<prs::PullRequestFetchResult>,
    ) {
        match result {
            Ok(mut result) => {
                let selected_id = self.get_selected_pr().map(|pr| pr.id.clone());
                self.replace_repo_prs(
                    &pr.repository.owner.login,
                    &pr.repository.name,
                    &mut result.prs,
                    selected_id,
                );
                self.drop_preview_cache_for(&pr.id);
                self.refresh_preview();
                let message = message.unwrap_or_else(|| {
                    if self.prs.iter().any(|p| p.id == pr.id) {
                        format!("Reloaded {}.", pr.numslug())
                    } else {
                        format!("Removed {} (no longer open).", pr.numslug())
                    }
                });
                self.set_reload_status(message, result.warning);
            }
            Err(err) => self.set_status(match message {
                Some(message) => format!("{message}. Reload failed: {err}"),
                None => format!("❌ Reload error for {}: {err}", pr.numslug()),
            }),
        }
    }

    fn apply_approval_result(&mut self, pr: PrNode, result: surf::Result<()>) {
        match result {
            Ok(()) => {
                let message = format!("✅ Approved PR {}.", pr.numslug());
                self.set_status(format!("{message} Reloading..."));
                self.queue_task(Task::ReloadRepo {
                    pr,
                    message: Some(message),
                });
            }
            Err(err) => self.set_status(format!("❌ Failed to approve PR {}: {err}", pr.numslug())),
        }
    }

    fn apply_contributions_result(
        &mut self,
        result: surf::Result<(
            crate::cmd::viewer::Profile,
            crate::cmd::contributions::model::Res,
        )>,
    ) {
        match result {
            Ok((profile, calendar)) => {
                self.viewer_profile_url = Some(profile.url);
                self.contributions.update(&calendar);
            }
            Err(err) => {
                self.contributions.show_load_error();
                self.set_status(format!("❌ Contrib load error: {err}"));
            }
        }
    }

    fn apply_preview_result(
        &mut self,
        pr: PrNode,
        mode: PreviewMode,
        result: surf::Result<Text<'static>>,
    ) {
        if !self.prs.iter().any(|p| p.id == pr.id) {
            return;
        }
        let visible = self.mode == AppMode::Prs
            && self.preview.mode == Some(mode)
            && self
                .get_selected_pr()
                .is_some_and(|selected| selected.id == pr.id);
        match result {
            Ok(text) => {
                self.cache.insert((mode, pr.id.clone()), text);
                if visible && self.status_clear_at.is_none() {
                    self.set_status(format!("✅ Loaded {mode} for #{}", pr.number));
                }
            }
            Err(err) => {
                let message = format!("Failed to load {mode} for #{}: {err}", pr.number);
                // Replace the loading placeholder; a manual reload can retry the request.
                self.cache
                    .insert((mode, pr.id), Text::from(message.clone()));
                if visible {
                    self.set_status(format!("❌ {message}"));
                }
            }
        }
    }

    fn apply_search_result(&mut self, revision: u64, result: surf::Result<Vec<SearchItem>>) {
        if revision != self.search.revision || self.mode != AppMode::Search {
            return;
        }
        match result {
            Ok(items) => {
                self.search.results = items;
                self.search.select_first();
                self.search.focus = SearchFocus::Results;
                self.search.preview_open = false;
                self.set_status(format!(
                    "✅ Search done. {} results.",
                    self.search.results.len()
                ));
            }
            Err(err) => self.set_status(format!("❌ Search error: {err}")),
        }
    }

    fn set_reload_status(
        &mut self,
        message: String,
        warning: Option<prs::DependabotAlertLookupWarning>,
    ) {
        if let Some(warning) = warning {
            self.set_status(format!("⚠️ {message} {}", warning.message()));
        } else {
            self.set_status(message);
        }
    }
}

impl App {
    // Set a status message that clears automatically after a short delay.
    fn set_status<T: Into<String>>(&mut self, msg: T) {
        self.status_message = Some(msg.into());
        self.status_clear_at = Some(Instant::now() + Duration::from_millis(3000));
    }

    // Set a status message that stays until explicitly replaced or cleared.
    fn set_status_persistent<T: Into<String>>(&mut self, msg: T) {
        self.status_message = Some(msg.into());
        self.status_clear_at = None;
    }
}

fn run_tui(specs: Vec<Slug>, auto_reload: Option<Duration>) -> io::Result<()> {
    let mut app = App::new(specs, auto_reload);
    terminal::run(|terminal| run_app(terminal, &mut app))
}

impl App {
    fn handle_key(&mut self, code: KeyCode) {
        match self.mode {
            AppMode::Prs => self.handle_key_prs(code),
            AppMode::Search => self.handle_key_search(code),
        }
    }

    fn handle_key_prs(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') => self.on_quit(),
            KeyCode::Char('s') => self.enter_search_mode(),
            KeyCode::Down | KeyCode::Char('j') => self.on_down(),
            KeyCode::Up | KeyCode::Char('k') => self.on_up(),
            KeyCode::Enter | KeyCode::Char('o') => self.on_open(),
            KeyCode::Char('p') => self.on_open_profile(),
            KeyCode::Char('m') => self.on_merge_key(),
            KeyCode::Char('a') => self.on_approve_key(),
            KeyCode::Char('r') => self.on_reload_key(),
            KeyCode::Char('R') => self.on_reload_all_key(),
            KeyCode::Char('c') => self.on_reload_contrib_key(),
            KeyCode::Char('?') => self.on_clear_help(),
            KeyCode::Right => self.on_right(),
            KeyCode::Left => self.on_left(),
            _ => {}
        }
    }

    fn handle_key_search(&mut self, code: KeyCode) {
        if is_search_back_key(self.search.focus, code) {
            self.exit_search_mode();
            return;
        }
        if self.search.focus == SearchFocus::Input
            && matches!(
                code,
                KeyCode::Up | KeyCode::Down | KeyCode::Backspace | KeyCode::Char(_)
            )
        {
            self.invalidate_search();
        }
        match self.search.focus {
            SearchFocus::Input => match code {
                KeyCode::Enter => self.on_search_submit(),
                KeyCode::Up => self.search.navigate_history(-1),
                KeyCode::Down => self.search.navigate_history(1),
                KeyCode::Backspace => {
                    self.search.reset_history_nav();
                    self.search.query.pop();
                }
                KeyCode::Char(ch) => {
                    self.search.reset_history_nav();
                    self.search.query.push(ch);
                }
                _ => {}
            },
            SearchFocus::Results => match code {
                KeyCode::Enter => self.open_search_result(),
                KeyCode::Char('c') => self.copy_search_result_repo(),
                KeyCode::Tab | KeyCode::Backspace => {
                    self.search.focus = SearchFocus::Input;
                    self.search.preview_open = false;
                }
                KeyCode::Right => self.search.preview_open = true,
                KeyCode::Left => self.search.preview_open = false,
                KeyCode::Up | KeyCode::Char('k') => self.search.navigate(-1),
                KeyCode::Down | KeyCode::Char('j') => self.search.navigate(1),
                _ => {}
            },
        }
    }

    fn handle_mouse(&mut self, kind: MouseEventKind) {
        if self.mode != AppMode::Prs {
            return;
        }
        match kind {
            MouseEventKind::ScrollDown => self.scroll_preview_down(3),
            MouseEventKind::ScrollUp => self.scroll_preview_up(3),
            _ => {}
        }
    }
    fn on_quit(&mut self) {
        self.should_quit = true;
    }

    fn on_down(&mut self) {
        if self.preview.mode.is_some() {
            self.scroll_preview_down(1);
        } else {
            self.navigate(1);
        }
    }

    fn on_up(&mut self) {
        if self.preview.mode.is_some() {
            self.scroll_preview_up(1);
        } else {
            self.navigate(-1);
        }
    }

    fn on_open(&mut self) {
        self.open_url();
    }

    fn on_open_profile(&mut self) {
        let Some(url) = self.viewer_profile_url.as_deref() else {
            self.set_status("No viewer profile loaded.");
            return;
        };
        if let Err(e) = open::that(url) {
            self.set_status(format!("❌ Failed to open profile: {}", e));
        }
    }

    fn open_search_result(&mut self) {
        let Some(item) = self.selected_search_result() else {
            return;
        };
        if let Err(e) = open::that(&item.html_url) {
            eprintln!("Failed to open URL: {}", e);
        }
    }

    fn selected_search_result(&self) -> Option<&SearchItem> {
        let idx = self.search.list_state.selected()?;
        self.search.results.get(idx)
    }

    fn copy_search_result_repo(&mut self) {
        let Some(repo) = self.selected_search_result().map(|item| item.repo.clone()) else {
            self.set_status("No search result selected.");
            return;
        };
        match copy_to_clipboard(&repo) {
            Ok(()) => self.set_status(format!("Copied repo slug: {}", repo)),
            Err(e) => self.set_status(format!("❌ Failed to copy repo slug: {}", e)),
        }
    }

    fn on_merge_key(&mut self) {
        if let Some(task) = &self.merge_task {
            self.set_status_persistent(format!(
                "Still waiting for merge of PR {}...",
                task.pr.numslug()
            ));
            return;
        }
        self.merge_selected();
    }

    fn on_approve_key(&mut self) {
        if let Some(pr) = self.get_selected_pr().cloned()
            && self.queue_task(Task::Approve(pr.clone()))
        {
            self.set_status_persistent(format!("Approving PR {}...", pr.numslug()));
        }
    }

    fn on_reload_key(&mut self) {
        if let Some(pr) = self.get_selected_pr().cloned() {
            self.set_status_persistent(format!("🔄 Reloading {}...", pr.numslug()));
            self.queue_task(Task::ReloadRepo { pr, message: None });
        }
    }

    fn on_reload_all_key(&mut self) {
        self.set_status_persistent("🔄 Reloading all...".to_string());
        self.queue_task(Task::Reload);
    }

    fn on_reload_contrib_key(&mut self) {
        self.set_status_persistent("🔄 Reloading contrib...".to_string());
        self.queue_task(Task::Contributions);
    }

    fn on_clear_help(&mut self) {
        self.status_message = None;
        self.status_clear_at = None;
    }

    fn queue_mode_if_needed(&mut self, mode: PreviewMode) -> bool {
        let Some(pr) = self.get_selected_pr().cloned() else {
            return false;
        };
        if self.cache.contains_key(&(mode, pr.id.clone())) {
            return false;
        }
        let number = pr.number;
        if self.queue_task(Task::Preview { pr, mode }) {
            self.set_status_persistent(format!("🔎 Loading {} for #{}...", mode, number));
            return true;
        }
        false
    }

    fn on_right(&mut self) {
        // Right: closed -> Body -> Diff -> Commits
        self.preview.scroll = 0;
        match self.preview.mode {
            None => {
                self.preview.mode = Some(PreviewMode::Body);
                self.queue_mode_if_needed(PreviewMode::Body);
            }
            Some(PreviewMode::Body) => {
                self.preview.mode = Some(PreviewMode::Diff);
                self.queue_mode_if_needed(PreviewMode::Diff);
            }
            Some(PreviewMode::Diff) => {
                self.preview.mode = Some(PreviewMode::Commits);
                self.queue_mode_if_needed(PreviewMode::Commits);
            }
            Some(PreviewMode::Commits) => {}
        }
    }

    fn on_left(&mut self) {
        // Left: Commits -> Diff -> Body -> Close
        self.preview.scroll = 0;
        match self.preview.mode {
            Some(PreviewMode::Commits) => {
                self.preview.mode = Some(PreviewMode::Diff);
                self.queue_mode_if_needed(PreviewMode::Diff);
            }
            Some(PreviewMode::Diff) => {
                self.preview.mode = Some(PreviewMode::Body);
                self.queue_mode_if_needed(PreviewMode::Body);
            }
            Some(PreviewMode::Body) => self.preview.mode = None,
            None => {}
        }
    }

    fn enter_search_mode(&mut self) {
        if self.search.owner.is_empty() {
            self.search.owner = Self::default_search_owner(&self.specs);
        }
        self.search.focus = SearchFocus::Input;
        self.search.preview_open = false;
        self.mode = AppMode::Search;
        self.set_status(format!(
            "Search user:{} (Enter to search)",
            self.search.owner
        ));
    }

    fn exit_search_mode(&mut self) {
        self.invalidate_search();
        self.mode = AppMode::Prs;
        self.search.preview_open = false;
        self.search.focus = SearchFocus::Input;
        self.status_message = None;
        self.status_clear_at = None;
        self.queue_preview_if_needed();
    }

    fn on_search_submit(&mut self) {
        let query = self.search.query.trim();
        if query.is_empty() {
            self.set_status("Enter search terms.".to_string());
            return;
        }
        let owner = self.search.owner.clone();
        let query = query.to_string();
        if self.search.remember_query(&query) {
            let _ = save_search_history(&search_history_path(), &self.search.history);
        }
        self.set_status_persistent(format!("🔎 Searching code for user:{}...", owner));
        self.search.revision += 1;
        self.queue_task(Task::Search {
            owner,
            query,
            revision: self.search.revision,
        });
    }

    fn invalidate_search(&mut self) {
        self.search.revision += 1;
        self.pending_tasks
            .retain(|task| !matches!(task, Task::Search { .. }));
        self.on_clear_help();
    }
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|f| view::ui(f, app))?;
        handle_terminal_events(app)?;
        if app.should_quit {
            break;
        }
        app.finish_pending_task();
        app.finish_merge();
        app.finish_auto_reload();
        app.start_pending_task();
        app.start_auto_reload();
        clear_expired_status(app);
    }
    Ok(())
}

fn handle_terminal_events(app: &mut App) -> io::Result<()> {
    if !event::poll(std::time::Duration::from_millis(100))? {
        return Ok(());
    }

    match event::read()? {
        Event::Key(key) => app.handle_key(key.code),
        Event::Mouse(m) => app.handle_mouse(m.kind),
        _ => {}
    }
    Ok(())
}

fn clear_expired_status(app: &mut App) {
    if let Some(clear_at) = app.status_clear_at
        && Instant::now() >= clear_at
    {
        app.status_message = None;
        app.status_clear_at = None;
    }
}

pub async fn run(slugs: Vec<String>, auto_reload_secs: u64) -> surf::Result<()> {
    let mut specs: Vec<Slug> = Vec::new();
    for slug in slugs {
        specs.push(Slug::try_from(slug.as_str())?);
    }

    let auto_reload = Some(Duration::from_secs(auto_reload_secs));
    run_tui(specs, auto_reload).map_err(|e| {
        surf::Error::from_str(
            surf::StatusCode::InternalServerError,
            format!("TUI error: {}", e),
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::styling;
    use ratatui::{style::Style, text::Line};

    fn test_pr(id: &str, number: usize) -> PrNode {
        serde_json::from_value(serde_json::json!({
            "repository": {
                "name": "repo",
                "owner": { "login": "owner" }
            },
            "number": number,
            "id": id,
            "title": format!("PR {number}"),
            "url": format!("https://github.com/owner/repo/pull/{number}"),
            "createdAt": "2026-01-01T00:00:00Z",
            "mergeStateStatus": "CLEAN",
            "reviewDecision": null,
            "commits": null,
            "reviewRequests": { "nodes": [] }
        }))
        .expect("test PR")
    }

    fn empty_app(mode: AppMode) -> App {
        App {
            prs: Vec::new(),
            pr_revision: 0,
            list_state: ListState::default(),
            should_quit: false,
            status_message: None,
            status_clear_at: None,
            specs: Vec::new(),
            cache: HashMap::new(),
            preview: Preview::default(),
            contributions: ContributionView::default(),
            viewer_profile_url: None,
            pending_tasks: VecDeque::new(),
            active_task: None,
            merge_task: None,
            auto_reload: None,
            mode,
            search: SearchState::new(String::default(), Vec::new()),
        }
    }

    fn test_contributions() -> (
        crate::cmd::viewer::Profile,
        crate::cmd::contributions::model::Res,
    ) {
        let calendar = serde_json::from_value(serde_json::json!({
            "data": {"user": {"contributionsCollection": {"contributionCalendar": {
                "totalContributions": 103,
                "weeks": [{"firstDay": "2026-05-10", "contributionDays": [
                    {"date": "2026-05-10", "contributionCount": 3, "color": "#40c463"},
                    {"date": "2026-05-11", "contributionCount": 100, "color": "#216e39"}
                ]}]
            }}}}
        }))
        .unwrap();
        (
            crate::cmd::viewer::Profile {
                login: "owner".into(),
                url: "https://example.test/owner".into(),
            },
            calendar,
        )
    }

    fn render_app(app: &mut App) -> ratatui::buffer::Buffer {
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(100, 15)).expect("terminal");
        terminal.draw(|frame| view::ui(frame, app)).expect("draw");
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn contributions_render_counts_colors_and_totals() {
        let mut app = empty_app(AppMode::Prs);
        app.apply_contributions_result(Ok(test_contributions()));

        let buffer = render_app(&mut app);
        let text = buffer_text(&buffer);
        assert!(text.contains("Contributions: total 103"));
        assert!(text.contains("# year to date:   103 51.50"));
        assert!(text.contains("# month to date:  103 51.50"));
        assert!(text.contains("# week to date:   103 51.50"));
        assert_eq!(buffer[(2, 4)].symbol(), "3");
        assert_eq!(buffer[(1, 4)].bg, Color::Rgb(64, 196, 99));
        assert_eq!(buffer[(1, 4)].fg, Color::Black);
        assert_eq!(buffer[(1, 5)].symbol(), "+");
        assert_eq!(buffer[(2, 5)].symbol(), "+");
        assert_eq!(buffer[(1, 5)].bg, Color::Rgb(33, 110, 57));
        assert_eq!(buffer[(1, 5)].fg, Color::White);
        assert_eq!(
            app.viewer_profile_url.as_deref(),
            Some("https://example.test/owner")
        );
    }

    #[test]
    fn contributions_show_initial_failure_and_recover_on_retry() {
        let mut app = empty_app(AppMode::Prs);
        assert!(buffer_text(&render_app(&mut app)).contains("Loading contributions..."));

        app.apply_contributions_result(Err(surf::Error::from_str(
            surf::StatusCode::ServiceUnavailable,
            "calendar unavailable",
        )));
        let text = buffer_text(&render_app(&mut app));
        assert!(text.contains("Could not load contributions. Press c to retry."));
        assert!(text.contains("Contrib load error: calendar unavailable"));

        app.apply_contributions_result(Ok(test_contributions()));
        let text = buffer_text(&render_app(&mut app));
        assert!(text.contains("Contributions: total 103"));
        assert!(!text.contains("Could not load contributions."));
    }

    #[test]
    fn contributions_keep_previous_calendar_when_refresh_fails() {
        let mut app = empty_app(AppMode::Prs);
        app.apply_contributions_result(Ok(test_contributions()));
        let before = render_app(&mut app);

        app.apply_contributions_result(Err(surf::Error::from_str(
            surf::StatusCode::ServiceUnavailable,
            "calendar unavailable",
        )));
        let after = render_app(&mut app);

        // Only the Help panel should change to report the refresh error.
        for y in 3..12 {
            for x in 0..100 {
                assert_eq!(before[(x, y)], after[(x, y)]);
            }
        }
        assert_eq!(
            app.viewer_profile_url.as_deref(),
            Some("https://example.test/owner")
        );
    }

    // Hold a worker response without sockets, sleeps, or live GitHub requests.
    fn hold_task(app: &mut App, task: Task) -> Sender<TaskEvent> {
        let (tx, rx) = async_std::channel::bounded(1);
        app.set_status_persistent(task.status());
        app.active_task = Some(RunningTask {
            task,
            rx,
            pr_revision: app.pr_revision,
        });
        tx
    }

    fn deliver(tx: &Sender<TaskEvent>, event: TaskEvent) {
        assert!(tx.try_send(event).is_ok());
    }

    fn fetched(prs: Vec<PrNode>) -> prs::PullRequestFetchResult {
        prs::PullRequestFetchResult { prs, warning: None }
    }

    #[test]
    fn pending_network_tasks_allow_navigation_search_and_quitting() {
        let pr = test_pr("one", 1);
        for task in [
            Task::Reload,
            Task::ReloadRepo {
                pr: pr.clone(),
                message: None,
            },
            Task::Approve(pr.clone()),
            Task::Contributions,
            Task::Preview {
                pr,
                mode: PreviewMode::Diff,
            },
            Task::Search {
                owner: "owner".into(),
                query: "rust".into(),
                revision: 0,
            },
        ] {
            let mut app = empty_app(AppMode::Prs);
            app.prs = vec![test_pr("one", 1), test_pr("two", 2)];
            app.list_state.select(Some(0));
            let _tx = hold_task(&mut app, task);

            app.finish_pending_task();
            app.handle_key(KeyCode::Char('j'));
            assert_eq!(app.get_selected_pr().unwrap().id, "two");
            app.handle_key(KeyCode::Char('s'));
            app.handle_key(KeyCode::Char('x'));
            assert_eq!(app.search.query, "x");
            app.handle_key(KeyCode::Esc);
            app.handle_key(KeyCode::Char('q'));
            assert!(app.should_quit);
            assert!(app.active_task.is_some());
        }
    }

    #[test]
    fn approval_keeps_its_target_and_reload_preserves_the_current_selection() {
        let mut app = empty_app(AppMode::Prs);
        let mut other = test_pr("two", 2);
        other.repository.name = "other".into();
        app.prs = vec![test_pr("one", 1), other];
        app.list_state.select(Some(0));
        app.on_approve_key();
        let task = app.pending_tasks.pop_front().unwrap();
        let Task::Approve(pr) = task.clone() else {
            panic!("approval expected");
        };
        let tx = hold_task(&mut app, task);
        app.on_approve_key();
        assert!(
            app.pending_tasks.is_empty(),
            "do not submit a duplicate review"
        );
        app.handle_key(KeyCode::Char('j'));

        deliver(
            &tx,
            TaskEvent::Approved {
                pr: pr.clone(),
                result: Ok(()),
            },
        );
        app.finish_pending_task();
        assert!(
            matches!(app.pending_tasks.front(), Some(Task::ReloadRepo { pr, .. }) if pr.id == "one")
        );
        app.apply_task_event(TaskEvent::RepoReloaded {
            pr: pr.clone(),
            message: None,
            result: Ok(fetched(vec![pr])),
        });
        assert_eq!(app.get_selected_pr().unwrap().id, "two");
    }

    #[test]
    fn editing_or_leaving_search_ignores_old_successes_and_errors() {
        for key in [KeyCode::Char('x'), KeyCode::Esc] {
            for failed in [false, true] {
                let mut app = empty_app(AppMode::Search);
                app.search.query = "rust".into();
                let tx = hold_task(
                    &mut app,
                    Task::Search {
                        owner: "owner".into(),
                        query: "rust".into(),
                        revision: 0,
                    },
                );
                app.handle_key(key);
                let result = if failed {
                    Err(surf::Error::from_str(
                        surf::StatusCode::BadGateway,
                        "old error",
                    ))
                } else {
                    Ok(vec![SearchItem {
                        repo: "owner/repo".into(),
                        path: "old.rs".into(),
                        html_url: String::new(),
                        matches: vec![],
                    }])
                };
                deliver(
                    &tx,
                    TaskEvent::SearchCompleted {
                        revision: 0,
                        result,
                    },
                );
                app.finish_pending_task();
                assert!(app.search.results.is_empty());
                assert_eq!(app.search.focus, SearchFocus::Input);
                assert!(app.status_message.is_none());
            }
        }
    }

    #[test]
    fn queued_searches_keep_only_the_latest_request() {
        let mut app = empty_app(AppMode::Search);
        for revision in 1..=3 {
            app.queue_task(Task::Search {
                owner: "owner".into(),
                query: revision.to_string(),
                revision,
            });
        }
        assert_eq!(app.pending_tasks.len(), 1);
        assert!(matches!(
            app.pending_tasks.front(),
            Some(Task::Search { revision: 3, .. })
        ));
    }

    #[test]
    fn search_queued_during_startup_uses_the_resolved_owner() {
        let mut app = empty_app(AppMode::Search);
        let tx = hold_task(&mut app, Task::Reload);
        app.queue_task(Task::Search {
            owner: String::default(),
            query: "rust".into(),
            revision: app.search.revision,
        });
        deliver(
            &tx,
            TaskEvent::Reloaded(Ok((vec![Slug::Owner("owner".into())], fetched(vec![])))),
        );

        app.finish_pending_task();

        assert_eq!(app.search.owner, "owner");
        assert!(
            matches!(app.pending_tasks.front(), Some(Task::Search { owner, query, .. })
            if owner == "owner" && query == "rust")
        );
    }

    #[test]
    fn stale_preview_does_not_repopulate_cache_after_a_list_update() {
        let mut app = empty_app(AppMode::Prs);
        let pr = test_pr("one", 1);
        app.prs = vec![pr.clone()];
        app.list_state.select(Some(0));
        app.preview.mode = Some(PreviewMode::Body);
        let tx = hold_task(
            &mut app,
            Task::Preview {
                pr: pr.clone(),
                mode: PreviewMode::Body,
            },
        );
        let mut updated = pr.clone();
        updated.title = "Updated title".into();
        app.apply_pr_list_and_restore_selection(vec![updated]);
        deliver(
            &tx,
            TaskEvent::PreviewLoaded {
                pr,
                mode: PreviewMode::Body,
                result: Ok(Text::from("old body")),
            },
        );

        app.finish_pending_task();

        assert!(app.cache.is_empty());
        assert!(
            matches!(app.pending_tasks.front(), Some(Task::Preview { pr, .. }) if pr.title == "Updated title")
        );
    }

    #[test]
    fn merge_completion_queues_followup_and_rejects_an_older_reload() {
        let mut app = empty_app(AppMode::Prs);
        let pr = test_pr("one", 1);
        app.prs = vec![pr.clone()];
        app.list_state.select(Some(0));
        let reload_tx = hold_task(&mut app, Task::Reload);
        let (merge_tx, rx) = async_std::channel::bounded(1);
        app.merge_task = Some(MergeTask { pr, rx });
        merge_tx.try_send(Ok(MergeOutcome::Merged)).unwrap();

        app.finish_merge();
        assert!(app.merge_task.is_none());
        assert!(
            matches!(app.pending_tasks.front(), Some(Task::ReloadRepo { pr, .. }) if pr.id == "one")
        );
        assert!(
            app.pending_tasks
                .iter()
                .any(|task| matches!(task, Task::Contributions))
        );
        deliver(
            &reload_tx,
            TaskEvent::Reloaded(Ok((vec![], fetched(vec![test_pr("outdated", 3)])))),
        );
        app.finish_pending_task();

        assert_eq!(app.prs[0].id, "one");
        assert!(app.status_message.as_ref().unwrap().contains("✅ Merged"));
    }

    #[test]
    fn merge_completion_refreshes_contributions_after_an_active_read() {
        for already_queued in [false, true] {
            let mut app = empty_app(AppMode::Prs);
            if already_queued {
                app.queue_task(Task::Contributions);
            }
            let contrib_tx = hold_task(&mut app, Task::Contributions);
            let (merge_tx, rx) = async_std::channel::bounded(1);
            app.merge_task = Some(MergeTask {
                pr: test_pr("one", 1),
                rx,
            });
            merge_tx.try_send(Ok(MergeOutcome::Merged)).unwrap();
            app.finish_merge();

            let calendar = serde_json::from_value(serde_json::json!({
                "data": {"user": {"contributionsCollection": {"contributionCalendar": {
                    "totalContributions": 1,
                    "weeks": [{"firstDay": "2026-11-01", "contributionDays": [
                        {"date": "2026-11-01", "contributionCount": 1, "color": "#40c463"}
                    ]}]
                }}}}
            }))
            .unwrap();
            deliver(
                &contrib_tx,
                TaskEvent::ContributionsLoaded(Ok((
                    crate::cmd::viewer::Profile {
                        login: "owner".into(),
                        url: "https://example.test/owner".into(),
                    },
                    calendar,
                ))),
            );
            app.finish_pending_task();

            assert_eq!(app.contributions.title, "Contributions: total 1");
            assert_eq!(
                app.pending_tasks
                    .iter()
                    .filter(|task| matches!(task, Task::Contributions))
                    .count(),
                1,
                "refresh the pre-merge calendar exactly once"
            );
        }
    }

    #[test]
    fn merge_completion_retries_a_superseded_reload_for_other_repositories() {
        for reload_all in [false, true] {
            let mut app = empty_app(AppMode::Prs);
            let pr = test_pr("one", 1);
            let mut merged = test_pr("two", 2);
            merged.repository.name = "other".into();
            app.prs = vec![pr.clone(), merged.clone()];
            app.list_state.select(Some(0));
            let task = if reload_all {
                Task::Reload
            } else {
                Task::ReloadRepo {
                    pr: pr.clone(),
                    message: Some("Approved PR #1.".into()),
                }
            };
            let reload_tx = hold_task(&mut app, task);
            let (merge_tx, rx) = async_std::channel::bounded(1);
            app.merge_task = Some(MergeTask { pr: merged, rx });
            merge_tx.try_send(Ok(MergeOutcome::Merged)).unwrap();
            app.finish_merge();
            let result = fetched(vec![pr.clone()]);
            let event = if reload_all {
                TaskEvent::Reloaded(Ok((vec![], result)))
            } else {
                TaskEvent::RepoReloaded {
                    pr: pr.clone(),
                    message: Some("Approved PR #1.".into()),
                    result: Ok(result),
                }
            };
            deliver(&reload_tx, event);
            app.finish_pending_task();

            let retry = app.pending_tasks.pop_back().expect("retry the stale read");
            if reload_all {
                assert!(matches!(retry, Task::Reload));
            } else {
                assert!(
                    matches!(&retry, Task::ReloadRepo { pr, message: Some(message) }
                    if pr.id == "one" && message == "Approved PR #1.")
                );
            }
            let retry_tx = hold_task(&mut app, retry);
            let mut updated = pr.clone();
            updated.review_decision = Some(prs::ReviewDecision::Approved);
            let result = fetched(vec![updated]);
            deliver(
                &retry_tx,
                if reload_all {
                    TaskEvent::Reloaded(Ok((vec![], result)))
                } else {
                    TaskEvent::RepoReloaded {
                        pr,
                        message: None,
                        result: Ok(result),
                    }
                },
            );
            app.finish_pending_task();
            assert_eq!(app.prs[0].review_status(), "[approved]");
        }
    }

    #[test]
    fn queued_preview_keeps_its_target_but_uses_updated_pr_details() {
        let mut app = empty_app(AppMode::Prs);
        let mut pr = test_pr("one", 1);
        let other = test_pr("two", 2);
        app.prs = vec![pr.clone(), other.clone()];
        app.list_state.select(Some(0));
        app.on_right();
        app.on_left();
        app.on_down();
        pr.title = "Updated title".into();

        app.apply_pr_list_and_restore_selection(vec![pr, other]);

        assert_eq!(app.get_selected_pr().unwrap().id, "two");
        let Some(Task::Preview { pr, mode }) = app.pending_tasks.pop_front() else {
            panic!("preview should remain queued");
        };
        let text = styling::prettify_pr_preview(&pr.title, &pr.url, "body");
        app.apply_task_event(TaskEvent::PreviewLoaded {
            pr,
            mode,
            result: Ok(text),
        });
        app.list_state.select(Some(0));
        app.on_right();
        assert!(
            build_preview_text(&app)
                .lines
                .iter()
                .any(|line| line_text(line).contains("Updated title"))
        );
    }

    #[test]
    fn queued_mutations_invalidate_in_flight_auto_reload() {
        let mut app = empty_app(AppMode::Prs);
        app.prs = vec![test_pr("one", 1)];
        let mut auto_reload = AutoReload::new(Duration::from_secs(60));
        auto_reload.in_flight = true;
        auto_reload.next_id = 2;
        assert!(
            auto_reload
                .tx
                .try_send(AutoReloadEvent {
                    id: 1,
                    result: Ok(fetched(vec![]))
                })
                .is_ok()
        );
        app.auto_reload = Some(auto_reload);
        app.queue_task(Task::Approve(app.prs[0].clone()));

        app.finish_auto_reload();

        assert_eq!(app.prs.len(), 1);
        assert!(!app.auto_reload.as_ref().unwrap().in_flight);
    }

    #[test]
    fn closed_task_channels_only_warn_about_retrying_approvals() {
        let pr = test_pr("one", 1);
        for task in [
            Task::Reload,
            Task::ReloadRepo {
                pr: pr.clone(),
                message: None,
            },
            Task::Approve(pr.clone()),
            Task::Contributions,
            Task::Preview {
                pr,
                mode: PreviewMode::Body,
            },
            Task::Search {
                owner: "owner".into(),
                query: "rust".into(),
                revision: 0,
            },
        ] {
            let mut app = empty_app(AppMode::Prs);
            let is_approval = matches!(task, Task::Approve(_));
            let tx = hold_task(&mut app, task);
            drop(tx);

            app.finish_pending_task();

            assert!(app.active_task.is_none());
            let message = app.status_message.unwrap();
            assert!(message.starts_with("❌ Background task stopped before returning a result."));
            assert_eq!(
                message.contains("Check GitHub before retrying an approval."),
                is_approval
            );
        }
    }

    #[test]
    fn preview_errors_replace_loading_and_do_not_change_other_previews() {
        let mut app = empty_app(AppMode::Prs);
        let pr = test_pr("one", 1);
        app.prs = vec![pr.clone()];
        app.list_state.select(Some(0));
        app.preview.mode = Some(PreviewMode::Diff);
        let tx = hold_task(
            &mut app,
            Task::Preview {
                pr: pr.clone(),
                mode: PreviewMode::Body,
            },
        );
        deliver(
            &tx,
            TaskEvent::PreviewLoaded {
                pr,
                mode: PreviewMode::Body,
                result: Err(surf::Error::from_str(surf::StatusCode::Forbidden, "denied")),
            },
        );
        app.finish_pending_task();
        assert_eq!(app.preview.mode, Some(PreviewMode::Diff));
        assert!(app.status_message.is_none());
        let text = &app.cache[&(PreviewMode::Body, "one".into())];
        assert!(line_text(&text.lines[0]).contains("denied"));
    }

    #[async_std::test]
    async fn pending_merge_allows_navigation_and_quitting_without_resubmitting() {
        let mut app = empty_app(AppMode::Prs);
        app.prs = vec![test_pr("one", 1), test_pr("two", 2)];
        app.list_state.select(Some(0));
        let (_tx, rx) = async_std::channel::bounded(1);
        app.merge_task = Some(MergeTask {
            pr: app.prs[0].clone(),
            rx,
        });

        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('m'));
        app.finish_merge();
        assert_eq!(app.get_selected_pr().unwrap().id, "two");
        assert!(app.pending_tasks.is_empty());
        assert!(
            app.status_message
                .as_ref()
                .unwrap()
                .contains("#1 in owner/repo")
        );
        app.handle_key(KeyCode::Char('q'));
        assert!(app.should_quit);
    }

    #[async_std::test]
    async fn merge_error_keeps_the_list_and_identifies_the_original_pr() {
        let mut app = empty_app(AppMode::Prs);
        app.prs = vec![test_pr("one", 1), test_pr("two", 2)];
        app.list_state.select(Some(1));
        let (tx, rx) = async_std::channel::bounded(1);
        app.merge_task = Some(MergeTask {
            pr: app.prs[0].clone(),
            rx,
        });
        tx.send(Err(surf::Error::from_str(
            surf::StatusCode::BadRequest,
            "Required checks failed",
        )))
        .await
        .unwrap();

        app.finish_merge();

        assert!(app.merge_task.is_none());
        assert_eq!(app.prs.len(), 2);
        assert_eq!(app.get_selected_pr().unwrap().id, "two");
        let status = app.status_message.unwrap();
        assert!(status.contains("#1 in owner/repo"));
        assert!(status.contains("Required checks failed"));
        assert!(!status.contains("✅ Merged"));
    }

    #[test]
    fn esc_leaves_search_input() {
        assert!(is_search_back_key(SearchFocus::Input, KeyCode::Esc));
    }

    #[test]
    fn q_leaves_search_results() {
        assert!(is_search_back_key(SearchFocus::Results, KeyCode::Char('q')));
    }

    #[test]
    fn q_stays_available_in_search_input() {
        assert!(!is_search_back_key(SearchFocus::Input, KeyCode::Char('q')));
    }

    #[test]
    fn search_input_help_mentions_common_filters() {
        let text = build_search_input_help("octocat");
        let joined = text
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("extension:yml"));
        assert!(joined.contains("path:.github/workflows"));
        assert!(joined.contains("filename:Dockerfile"));
        assert!(joined.contains("language:rust"));
        assert!(joined.contains("repo:owner/repo"));
        assert!(joined.contains("user:octocat"));
    }

    #[test]
    fn search_results_help_mentions_copy_slug() {
        assert!(search_help_base(SearchFocus::Results).contains("c:copy slug"));
    }

    #[test]
    fn search_input_help_mentions_history() {
        assert!(search_help_base(SearchFocus::Input).contains("history"));
    }

    #[test]
    fn prs_help_mentions_profile_shortcut() {
        assert!(prs_help_base().contains("p:profile"));
    }

    #[test]
    fn pr_list_line_skips_empty_optional_fields() {
        let pr = test_pr("one", 1);
        let rendered = line_text(&pr_list_line(&pr));

        assert_eq!(rendered, "#1 ✅ owner/repo PR 1 (2026-01-01)");
    }

    #[test]
    fn pr_list_line_highlights_review_decision() {
        for (decision, badge, color) in [
            (prs::ReviewDecision::Approved, "[approved]", Color::Magenta),
            (
                prs::ReviewDecision::ChangesRequested,
                "[changes requested]",
                Color::Red,
            ),
            (
                prs::ReviewDecision::ReviewRequired,
                "[review required]",
                Color::Yellow,
            ),
        ] {
            let mut pr = test_pr("one", 1);
            pr.review_decision = Some(decision);
            let line = pr_list_line(&pr);

            assert_eq!(span_style(&line, badge), Style::default().fg(color));
        }
    }

    #[test]
    fn pr_list_line_highlights_review_requests() {
        let pr: PrNode = serde_json::from_value(serde_json::json!({
            "repository": {
                "name": "repo",
                "owner": { "login": "owner" }
            },
            "number": 1,
            "id": "one",
            "title": "PR 1",
            "url": "https://github.com/owner/repo/pull/1",
            "createdAt": "2026-01-01T00:00:00Z",
            "mergeStateStatus": "CLEAN",
            "reviewDecision": null,
            "commits": null,
            "reviewRequests": {
                "nodes": [{
                    "requestedReviewer": {
                        "__typename": "User",
                        "login": "alice"
                    }
                }]
            }
        }))
        .expect("test PR");
        let line = pr_list_line(&pr);

        assert_eq!(
            span_style(&line, "[r: alice]"),
            Style::default().fg(Color::Yellow)
        );
    }

    #[test]
    fn pr_list_marks_dependabot_alert_origin() {
        let mut app = empty_app(AppMode::Prs);
        let mut pr = test_pr("one", 1);
        pr.dependabot_alert_origin = true;
        app.prs = vec![pr];

        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(120, 5)).expect("terminal");
        terminal
            .draw(|frame| render_pr_list(frame, &mut app, frame.area()))
            .expect("draw");

        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("[dep-alert]"));
        let buffer = terminal.backend().buffer();
        let alert_x = find_row_text_x(buffer, 1, prs::DEPENDABOT_ALERT_BADGE);
        assert_eq!(buffer[(1, 1)].fg, Color::Green);
        for x in alert_x..alert_x + prs::DEPENDABOT_ALERT_BADGE.len() as u16 {
            assert_eq!(buffer[(x, 1)].fg, Color::Cyan);
        }
        assert_eq!(
            buffer[(alert_x + prs::DEPENDABOT_ALERT_BADGE.len() as u16, 1)].fg,
            Color::Green
        );
    }

    fn find_row_text_x(buffer: &ratatui::buffer::Buffer, y: u16, needle: &str) -> u16 {
        let symbols = needle.chars().map(|c| c.to_string()).collect::<Vec<_>>();
        let max_x = buffer.area.width.saturating_sub(symbols.len() as u16);
        for x in 0..=max_x {
            if symbols
                .iter()
                .enumerate()
                .all(|(i, symbol)| buffer[(x + i as u16, y)].symbol() == symbol)
            {
                return x;
            }
        }
        panic!("{needle:?} not found on row {y}");
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    fn span_style(line: &Line<'_>, text: &str) -> Style {
        line.spans
            .iter()
            .find(|span| span.content.as_ref() == text)
            .unwrap_or_else(|| panic!("{text:?} not found in {:?}", line_text(line)))
            .style
    }

    #[test]
    fn search_history_keeps_latest_unique_queries() {
        let mut history = Vec::new();
        assert!(push_search_history(&mut history, " first "));
        assert!(push_search_history(&mut history, "second"));
        assert!(push_search_history(&mut history, "first"));

        assert_eq!(history, vec!["second".to_string(), "first".to_string()]);
    }

    #[test]
    fn search_history_navigation_restores_draft() {
        let mut state = SearchState::new(
            String::default(),
            vec!["first".to_string(), "second".to_string()],
        );
        state.query = "draft".to_string();

        state.navigate_history(-1);
        assert_eq!(state.query, "second");
        state.navigate_history(-1);
        assert_eq!(state.query, "first");
        state.navigate_history(1);
        assert_eq!(state.query, "second");
        state.navigate_history(1);
        assert_eq!(state.query, "draft");
        state.navigate_history(1);
        assert_eq!(state.query, "draft");
    }

    #[test]
    fn search_history_down_without_selection_keeps_query() {
        let mut state = SearchState::new(String::default(), vec!["first".to_string()]);
        state.query = "draft".to_string();

        state.navigate_history(1);

        assert_eq!(state.query, "draft");
    }

    #[test]
    fn search_history_round_trips_file() -> io::Result<()> {
        let path =
            std::env::temp_dir().join(format!("gh-chk-search-history-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let history = vec!["first".to_string(), "second".to_string()];

        save_search_history(&path, &history)?;

        assert_eq!(load_search_history(&path), history);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn reloading_preserves_selected_pr_by_id() {
        let mut app = empty_app(AppMode::Prs);
        app.prs = vec![test_pr("one", 1), test_pr("two", 2)];
        app.list_state.select(Some(1));

        app.apply_pr_list_and_restore_selection(vec![test_pr("two", 2), test_pr("one", 1)]);

        assert_eq!(app.list_state.selected(), Some(0));
        assert_eq!(app.get_selected_pr().map(|pr| pr.id.as_str()), Some("two"));
    }

    #[test]
    fn deferring_auto_reload_keeps_in_flight_result_current() {
        let mut app = empty_app(AppMode::Prs);
        let mut auto_reload = AutoReload::new(Duration::from_secs(60));
        auto_reload.in_flight = true;
        auto_reload.next_id = 2;
        app.auto_reload = Some(auto_reload);

        app.defer_auto_reload();

        let auto_reload = app.auto_reload.as_ref().expect("auto-reload");
        assert_eq!(auto_reload.ignore_before_id, 0);
    }

    #[test]
    fn auto_reload_queues_preview_for_fallback_selection() {
        let mut app = empty_app(AppMode::Prs);
        app.prs = vec![test_pr("one", 1)];
        app.list_state.select(Some(0));
        app.preview.mode = Some(PreviewMode::Body);

        let mut auto_reload = AutoReload::new(Duration::from_secs(60));
        auto_reload.in_flight = true;
        async_std::task::block_on(auto_reload.tx.send(AutoReloadEvent {
            id: 1,
            result: Ok(prs::PullRequestFetchResult {
                prs: vec![test_pr("two", 2)],
                warning: None,
            }),
        }))
        .expect("send auto-reload event");
        app.auto_reload = Some(auto_reload);

        app.finish_auto_reload();

        assert_eq!(app.get_selected_pr().map(|pr| pr.id.as_str()), Some("two"));
        assert!(matches!(
            app.pending_tasks.front(),
            Some(Task::Preview {
                mode: PreviewMode::Body,
                ..
            })
        ));
    }

    #[test]
    fn leaving_search_queues_preview_after_auto_reload() {
        let mut app = empty_app(AppMode::Search);
        app.prs = vec![test_pr("one", 1)];
        app.list_state.select(Some(0));
        app.preview.mode = Some(PreviewMode::Body);

        app.apply_pr_list_and_restore_selection(vec![test_pr("two", 2)]);
        app.exit_search_mode();

        assert_eq!(app.mode, AppMode::Prs);
        assert_eq!(app.get_selected_pr().map(|pr| pr.id.as_str()), Some("two"));
        assert!(matches!(
            app.pending_tasks.front(),
            Some(Task::Preview {
                mode: PreviewMode::Body,
                ..
            })
        ));
    }

    #[test]
    fn auto_reload_waits_interval_after_completion() {
        let mut app = empty_app(AppMode::Prs);
        let mut auto_reload = AutoReload::new(Duration::from_secs(60));
        auto_reload.in_flight = true;
        auto_reload.next_at = Instant::now();
        async_std::task::block_on(auto_reload.tx.send(AutoReloadEvent {
            id: 1,
            result: Ok(prs::PullRequestFetchResult {
                prs: Vec::new(),
                warning: None,
            }),
        }))
        .expect("send auto-reload event");
        app.auto_reload = Some(auto_reload);

        app.finish_auto_reload();

        let auto_reload = app.auto_reload.as_ref().expect("auto-reload");
        assert!(!auto_reload.in_flight);
        assert!(auto_reload.next_at > Instant::now());
    }

    #[async_std::test]
    async fn c_without_selection_sets_status_in_search_results() {
        let mut app = empty_app(AppMode::Search);
        app.search.focus = SearchFocus::Results;

        app.handle_key_search(KeyCode::Char('c'));

        assert_eq!(
            app.status_message.as_deref(),
            Some("No search result selected.")
        );
    }
}
