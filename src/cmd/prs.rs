use crate::rest;
use colored::Colorize;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::fmt::Display;
use std::io;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
enum RequestedReviewer {
    User { login: String },
    Team { name: String },
}

impl std::fmt::Display for RequestedReviewer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestedReviewer::User { login } => write!(f, "{}", login),
            RequestedReviewer::Team { name } => write!(f, "team:{}", name),
        }
    }
}

fn extract_reviewer_names(
    review_requests: &repository::pull_requests::nodes::review_requests::ReviewRequests,
) -> Vec<String> {
    review_requests
        .nodes
        .iter()
        .filter_map(|node| node.requested_reviewer.as_ref().map(ToString::to_string))
        .collect()
}

nestruct::nest! {
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    Repository {
        name: String,
        pull_requests: {
            nodes: [{
                id: String,
                number: usize,
                title: String,
                url: String,
                merge_state_status: crate::cmd::prs::MergeStateStatus,
                review_requests: {
                    nodes: [{
                        requested_reviewer: Option<crate::cmd::prs::RequestedReviewer>,
                    }]
                }
            }]
        }
    }
}

nestruct::nest! {
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    Res {
        data: {
            repository_owner: {
                repositories: {
                    nodes: [ crate::cmd::prs::repository::Repository ]
                }
            }
        }
    }
}

nestruct::nest! {
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    RepoRes {
        data: {
            repository_owner: {
                repository: crate::cmd::prs::repository::Repository
            }
        }
    }
}

nestruct::nest! {
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    #[serde(rename_all = "camelCase")]
    PrBodyRes {
        data: {
            repository: {
                pull_request: {
                    body_text: String,
                }
            }
        }
    }
}

impl Display for repository::pull_requests::nodes::Nodes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = format!(
            "{:>6} {} {} {}",
            format!("#{}", self.number).bold(),
            self.merge_state_status.to_emoji(),
            self.url,
            self.title.bold()
        );
        write!(f, "{}", self.merge_state_status.colorize(&s))
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum MergeStateStatus {
    Behind,
    Blocked,
    Clean,
    Dirty,
    Draft,
    HasHooks,
    Unknown,
    Unstable,
}

impl MergeStateStatus {
    fn to_emoji(&self) -> String {
        match self {
            Self::Behind => "⏩",
            Self::Blocked => "🚫",
            Self::Clean => "✅",
            Self::Dirty => "⚠️ ",
            Self::Draft => "✏️ ",
            Self::HasHooks => "🪝",
            Self::Unknown => "❓",
            Self::Unstable => "❌",
        }
        .to_owned()
    }

    fn colorize(&self, s: &str) -> String {
        match self {
            Self::Behind => s.yellow(),
            Self::Blocked => s.red(),
            Self::Clean => s.green(),
            Self::Dirty => s.yellow(),
            Self::Draft => s.white(),
            Self::HasHooks => s.yellow(),
            Self::Unknown => s.magenta(),
            Self::Unstable => s.yellow(),
        }
        .to_string()
    }
}

#[derive(Debug, Clone)]
enum SlugSpec {
    Owner(String),
    Repo { owner: String, name: String },
}

async fn merge_pr(pr_id: &str) -> surf::Result<()> {
    let v = json!({ "pullRequestId": pr_id });
    let q = json!({ "query": include_str!("../query/merge.pr.graphql"), "variables": v });
    crate::graphql::query::<serde_json::Value>(&q).await?;
    Ok(())
}

pub async fn check(slugs: Vec<String>, merge: bool, tui: bool) -> surf::Result<()> {
    let slugs = if slugs.is_empty() {
        vec![crate::cmd::viewer::get().await?]
    } else {
        slugs
    };

    if tui {
        // Build slug specs and initial PR list
        let mut specs: Vec<SlugSpec> = Vec::new();
        let mut all_prs = Vec::new();
        for slug in slugs.clone() {
            let vs: Vec<String> = slug.split('/').map(String::from).collect();
            match vs.len() {
                1 => {
                    specs.push(SlugSpec::Owner(vs[0].clone()));
                    let prs = fetch_owner_prs(&vs[0]).await?;
                    all_prs.extend(prs);
                }
                2 => {
                    specs.push(SlugSpec::Repo {
                        owner: vs[0].clone(),
                        name: vs[1].clone(),
                    });
                    let prs = fetch_repo_prs(&vs[0], &vs[1]).await?;
                    all_prs.extend(prs);
                }
                _ => panic!("unknown slug format"),
            }
        }
        run_tui(all_prs, specs).map_err(|e| {
            surf::Error::from_str(
                surf::StatusCode::InternalServerError,
                format!("TUI error: {}", e),
            )
        })?;
    } else {
        for slug in slugs {
            println!("{}", slug.bright_blue());
            let vs: Vec<String> = slug.split('/').map(String::from).collect();
            match vs.len() {
                1 => check_owner(&vs[0], merge).await?,
                2 => check_repo(&vs[0], &vs[1], merge).await?,
                _ => panic!("unknown slug format"),
            }
        }
    }
    Ok(())
}

async fn check_owner(owner: &str, merge: bool) -> surf::Result<()> {
    let v = json!({ "login": owner });
    let q = json!({ "query": include_str!("../query/prs.graphql"), "variables": v });
    let res = crate::graphql::query::<res::Res>(&q).await?;
    match crate::config::FORMAT.get() {
        Some(&crate::config::Format::Json) => println!("{}", serde_json::to_string_pretty(&res)?),
        _ => print_owner_text(&res, merge).await?,
    }
    Ok(())
}

async fn print_owner_text(res: &res::Res, merge: bool) -> surf::Result<()> {
    let mut count = 0usize;
    for repo in &res.data.repository_owner.repositories.nodes {
        if repo.pull_requests.nodes.is_empty() {
            continue;
        }
        println!("{}", repo.name.cyan());
        for pr in &repo.pull_requests.nodes {
            count += 1;
            println!("{pr}");
            if merge && pr.merge_state_status == MergeStateStatus::Clean {
                println!("🔄 Merging PR #{}", pr.number);
                merge_pr(&pr.id).await?;
                println!("✅ Merged PR #{}", pr.number);
            }
        }
    }
    println!("Count of PRs: {count}");
    Ok(())
}

async fn check_repo(owner: &str, name: &str, merge: bool) -> surf::Result<()> {
    let v = json!({ "login": owner, "name": name });
    let q = json!({ "query": include_str!("../query/prs.repo.graphql"), "variables": v });
    let res = crate::graphql::query::<repo_res::RepoRes>(&q).await?;
    match crate::config::FORMAT.get() {
        Some(&crate::config::Format::Json) => println!("{}", serde_json::to_string_pretty(&res)?),
        _ => print_repo_text(&res, merge).await?,
    }
    Ok(())
}

async fn print_repo_text(res: &repo_res::RepoRes, merge: bool) -> surf::Result<()> {
    let mut count = 0usize;
    for pr in &res.data.repository_owner.repository.pull_requests.nodes {
        count += 1;
        println!("{pr}");
        if merge && pr.merge_state_status == MergeStateStatus::Clean {
            println!("🔄 Merging PR #{}", pr.number);
            merge_pr(&pr.id).await?;
            println!("✅ Merged PR #{}", pr.number);
        }
    }
    println!("Count of PRs: {count}");
    Ok(())
}

#[derive(Debug, Clone)]
struct PrData {
    pub id: String,
    pub number: usize,
    pub title: String,
    pub url: String,
    pub slug: String,
    pub merge_state_status: MergeStateStatus,
    pub reviewers: Vec<String>,
}

impl PrData {
    pub fn display_line(&self) -> String {
        let reviewers_str = if self.reviewers.is_empty() {
            String::default()
        } else {
            format!(" 👥 {}", self.reviewers.join(", "))
        };
        format!(
            "{} {} {} {}{}",
            format!("#{}", self.number),
            self.merge_state_status.to_emoji(),
            self.slug,
            self.title,
            reviewers_str
        )
    }

    pub fn get_color(&self) -> Color {
        match self.merge_state_status {
            MergeStateStatus::Behind => Color::Yellow,
            MergeStateStatus::Blocked => Color::Red,
            MergeStateStatus::Clean => Color::Green,
            MergeStateStatus::Dirty => Color::Yellow,
            MergeStateStatus::Draft => Color::White,
            MergeStateStatus::HasHooks => Color::Yellow,
            MergeStateStatus::Unknown => Color::Magenta,
            MergeStateStatus::Unstable => Color::Yellow,
        }
    }
}

async fn fetch_owner_prs(owner: &str) -> surf::Result<Vec<PrData>> {
    let v = json!({ "login": owner });
    let q = json!({ "query": include_str!("../query/prs.graphql"), "variables": v });
    let res = crate::graphql::query::<res::Res>(&q).await?;

    let mut prs = Vec::new();
    for repo in &res.data.repository_owner.repositories.nodes {
        for pr in &repo.pull_requests.nodes {
            prs.push(PrData {
                id: pr.id.clone(),
                number: pr.number,
                title: pr.title.clone(),
                url: pr.url.clone(),
                slug: format!("{}/{}", owner, repo.name),
                merge_state_status: pr.merge_state_status.clone(),
                reviewers: extract_reviewer_names(&pr.review_requests),
            });
        }
    }
    Ok(prs)
}

async fn fetch_repo_prs(owner: &str, name: &str) -> surf::Result<Vec<PrData>> {
    let v = json!({ "login": owner, "name": name });
    let q = json!({ "query": include_str!("../query/prs.repo.graphql"), "variables": v });
    let res = crate::graphql::query::<repo_res::RepoRes>(&q).await?;

    let mut prs = Vec::new();
    for pr in &res.data.repository_owner.repository.pull_requests.nodes {
        prs.push(PrData {
            id: pr.id.clone(),
            number: pr.number,
            title: pr.title.clone(),
            url: pr.url.clone(),
            slug: format!("{}/{}", owner, name),
            merge_state_status: pr.merge_state_status.clone(),
            reviewers: extract_reviewer_names(&pr.review_requests),
        });
    }
    Ok(prs)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewMode {
    Body,
    Diff,
}

struct App {
    prs: Vec<PrData>,
    list_state: ListState,
    should_quit: bool,
    status_message: Option<String>,
    specs: Vec<SlugSpec>,
    preview_open: bool,
    preview_cache: HashMap<String, String>,
    diff_cache: HashMap<String, String>,
    preview_mode: PreviewMode,
    preview_scroll: u16,
    preview_area_height: u16,
}

impl App {
    fn new(prs: Vec<PrData>, specs: Vec<SlugSpec>) -> App {
        let mut list_state = ListState::default();
        if !prs.is_empty() {
            list_state.select(Some(0));
        }
        App {
            prs,
            list_state,
            should_quit: false,
            status_message: None,
            specs,
            preview_open: false,
            preview_cache: HashMap::new(),
            diff_cache: HashMap::new(),
            preview_mode: PreviewMode::Body,
            preview_scroll: 0,
            preview_area_height: 0,
        }
    }

    fn next(&mut self) {
        if self.prs.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= self.prs.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
        self.preview_scroll = 0;
    }

    fn previous(&mut self) {
        if self.prs.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.prs.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
        self.preview_scroll = 0;
    }

    fn get_selected_pr(&self) -> Option<&PrData> {
        if let Some(i) = self.list_state.selected() {
            self.prs.get(i)
        } else {
            None
        }
    }

    async fn merge_selected(&mut self) {
        if let Some(selected_index) = self.list_state.selected() {
            if let Some(pr) = self.prs.get(selected_index).cloned() {
                if pr.merge_state_status == MergeStateStatus::Clean {
                    self.status_message =
                        Some(format!("Merging PR #{} in {}...", pr.number, pr.slug));
                    match merge_pr(&pr.id).await {
                        Ok(_) => {
                            self.status_message =
                                Some(format!("✅ Merged PR #{} in {}", pr.number, pr.slug));
                            // Remove the merged PR from the list
                            self.prs.remove(selected_index);
                            // Adjust selection after removal
                            if self.prs.is_empty() {
                                self.list_state.select(None);
                            } else if selected_index >= self.prs.len() {
                                self.list_state.select(Some(self.prs.len() - 1));
                            }
                            // Keep the current selection index if it's still valid
                        }
                        Err(e) => {
                            self.status_message = Some(format!(
                                "❌ Failed to merge PR #{} in {}: {}",
                                pr.number, pr.slug, e
                            ));
                        }
                    }
                } else {
                    self.status_message = Some(format!(
                        "Cannot merge PR #{} in {}: not in clean state",
                        pr.number, pr.slug
                    ));
                }
            }
        }
    }

    fn open_url(&self) {
        if let Some(pr) = self.get_selected_pr() {
            if let Err(e) = open::that(&pr.url) {
                eprintln!("Failed to open URL: {}", e);
            }
        }
    }

    fn selected_key(&self) -> Option<String> {
        self.get_selected_pr().map(|p| p.id.clone())
    }

    async fn toggle_preview(&mut self) {
        self.preview_open = !self.preview_open;
        if self.preview_open {
            self.preview_scroll = 0;
            // Ensure current selection's preview is loaded
            if let Some(pr) = self.get_selected_pr().cloned() {
                if !self.preview_cache.contains_key(&pr.id) {
                    if let Err(e) = self.load_preview_for(&pr).await {
                        self.status_message = Some(format!(
                            "❌ Failed to load preview for #{} in {}: {}",
                            pr.number, pr.slug, e
                        ));
                    }
                }
            }
        }
    }

    async fn maybe_prefetch_on_move(&mut self) {
        if !self.preview_open {
            return;
        }
        if let Some(pr) = self.get_selected_pr().cloned() {
            if !self.preview_cache.contains_key(&pr.id) {
                let _ = self.load_preview_for(&pr).await;
            }
            if self.preview_mode == PreviewMode::Diff && !self.diff_cache.contains_key(&pr.id) {
                let _ = self.load_diff_for(&pr).await;
            }
        }
    }

    async fn load_preview_for(&mut self, pr: &PrData) -> surf::Result<()> {
        self.status_message = Some(format!("🔎 Loading preview for #{}...", pr.number));
        let (owner, name) = match pr.slug.split_once('/') {
            Some((o, n)) => (o.to_string(), n.to_string()),
            None => return Ok(()),
        };
        let body = fetch_pr_body(&owner, &name, pr.number).await?;
        self.preview_cache.insert(pr.id.clone(), body);
        self.status_message = Some(format!("✅ Loaded preview for #{}", pr.number));
        Ok(())
    }

    async fn load_diff_for(&mut self, pr: &PrData) -> surf::Result<()> {
        self.status_message = Some(format!("🔎 Loading diff for #{}...", pr.number));
        let (owner, name) = match pr.slug.split_once('/') {
            Some((o, n)) => (o.to_string(), n.to_string()),
            None => return Ok(()),
        };
        let files = fetch_pr_files(&owner, &name, pr.number).await?;
        let mut out = String::new();
        for f in files {
            out.push_str(&format!(
                "=== {} (+{}, -{}) ===\n",
                f.filename, f.additions, f.deletions
            ));
            match f.patch {
                Some(p) => {
                    out.push_str(&p);
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                }
                None => out.push_str("(no textual diff available)\n"),
            }
            out.push('\n');
        }
        if out.is_empty() {
            out = "No file changes found.".to_string();
        }
        self.diff_cache.insert(pr.id.clone(), out);
        self.status_message = Some(format!("✅ Loaded diff for #{}", pr.number));
        Ok(())
    }

    async fn switch_preview_mode(&mut self, mode: PreviewMode) {
        self.preview_mode = mode;
        if !self.preview_open {
            self.preview_open = true;
        }
        self.preview_scroll = 0;
        if let Some(pr) = self.get_selected_pr().cloned() {
            match mode {
                PreviewMode::Body => {
                    if !self.preview_cache.contains_key(&pr.id) {
                        let _ = self.load_preview_for(&pr).await;
                    }
                }
                PreviewMode::Diff => {
                    if !self.diff_cache.contains_key(&pr.id) {
                        let _ = self.load_diff_for(&pr).await;
                    }
                }
            }
        }
    }

    fn scroll_preview_down(&mut self, n: u16) {
        if self.preview_open {
            self.preview_scroll = self.preview_scroll.saturating_add(n);
        }
    }

    fn scroll_preview_up(&mut self, n: u16) {
        if self.preview_open {
            self.preview_scroll = self.preview_scroll.saturating_sub(n);
        }
    }

    async fn reload(&mut self) {
        self.status_message = Some("🔄 Reloading...".to_string());
        let mut new_list: Vec<PrData> = Vec::new();
        let mut any_err: Option<String> = None;
        for spec in self.specs.clone() {
            match spec {
                SlugSpec::Owner(owner) => match fetch_owner_prs(&owner).await {
                    Ok(mut prs) => new_list.append(&mut prs),
                    Err(e) => any_err = Some(format!("Failed to fetch {}: {}", owner, e)),
                },
                SlugSpec::Repo { owner, name } => match fetch_repo_prs(&owner, &name).await {
                    Ok(mut prs) => new_list.append(&mut prs),
                    Err(e) => any_err = Some(format!("Failed to fetch {}/{}: {}", owner, name, e)),
                },
            }
        }
        if let Some(err) = any_err {
            self.status_message = Some(format!("❌ Reload error: {}", err));
        } else {
            // Preserve selection index as best effort
            let sel = self.list_state.selected().unwrap_or(0);
            self.prs = new_list;
            if self.prs.is_empty() {
                self.list_state.select(None);
            } else {
                let new_sel = sel.min(self.prs.len().saturating_sub(1));
                self.list_state.select(Some(new_sel));
            }
            self.status_message = Some(format!("✅ Reloaded. {} PRs.", self.prs.len()));
        }
    }
}

fn run_tui(prs: Vec<PrData>, specs: Vec<SlugSpec>) -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(prs, specs);
    let res = async_std::task::block_on(run_app(&mut terminal, &mut app));

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    match key.code {
                        KeyCode::Char('q') => {
                            app.should_quit = true;
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if app.preview_open {
                                app.scroll_preview_down(1);
                            } else {
                                app.next();
                                app.maybe_prefetch_on_move().await;
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if app.preview_open {
                                app.scroll_preview_up(1);
                            } else {
                                app.previous();
                                app.maybe_prefetch_on_move().await;
                            }
                        }
                        KeyCode::Enter | KeyCode::Char('o') => {
                            app.open_url();
                        }
                        KeyCode::Char('m') => {
                            app.merge_selected().await;
                        }
                        KeyCode::Char('r') => {
                            app.reload().await;
                        }
                        KeyCode::Char('p') => {
                            app.toggle_preview().await;
                        }
                        KeyCode::Char('?') => {
                            // Clear status to show the default help instructions again
                            app.status_message = None;
                        }
                        KeyCode::Char('d') => {
                            app.switch_preview_mode(PreviewMode::Diff).await;
                        }
                        KeyCode::Char('b') => {
                            app.switch_preview_mode(PreviewMode::Body).await;
                        }
                        _ => {}
                    }
                }
                Event::Mouse(m) => match m.kind {
                    MouseEventKind::ScrollDown => {
                        app.scroll_preview_down(3);
                    }
                    MouseEventKind::ScrollUp => {
                        app.scroll_preview_up(3);
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn make_diff_text(diff: &str) -> Text {
    let mut text = Text::default();
    for line in diff.lines() {
        let styled = if line.starts_with("===") {
            Line::from(Span::styled(
                line.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
        } else if line.starts_with("@@") {
            Line::from(Span::styled(
                line.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ))
        } else if line.starts_with('+') {
            Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Green),
            ))
        } else if line.starts_with('-') {
            Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Red),
            ))
        } else {
            Line::from(line.to_string())
        };
        text.lines.push(styled);
    }
    text
}

fn ui(f: &mut Frame, app: &mut App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)].as_ref())
        .split(f.area());

    // If preview is open, split the main area horizontally
    let main_chunks = if app.preview_open {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
            .split(outer[0])
    } else {
        vec![outer[0]].into()
    };

    let items: Vec<ListItem> = app
        .prs
        .iter()
        .map(|pr| {
            let line = pr.display_line();
            ListItem::new(Line::from(Span::styled(
                line,
                Style::default().fg(pr.get_color()),
            )))
        })
        .collect();

    let items = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Pull Requests"),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    f.render_stateful_widget(items, main_chunks[0], &mut app.list_state);

    // Render preview panel if open
    if app.preview_open {
        let preview_text: Text = if let Some(pr) = app.get_selected_pr() {
            match app.preview_mode {
                PreviewMode::Body => match app.preview_cache.get(&pr.id) {
                    Some(body) => Text::from(format!("{}\n{}\n\n{}", pr.title, pr.url, body)),
                    None => Text::from("Loading preview..."),
                },
                PreviewMode::Diff => match app.diff_cache.get(&pr.id) {
                    Some(diff) => {
                        // Prepend a header line for context
                        let header = format!("Diff for #{} {}", pr.number, pr.slug);
                        let mut text = Text::from(header);
                        text.lines.push(Line::from(""));
                        let mut colored = make_diff_text(diff);
                        text.lines.append(&mut colored.lines);
                        text
                    }
                    None => Text::from("Loading diff..."),
                },
            }
        } else {
            Text::from("No selection")
        };

        let preview = Paragraph::new(preview_text)
            .block(Block::default().borders(Borders::ALL).title("Preview"))
            .wrap(Wrap { trim: false })
            .scroll((app.preview_scroll, 0));
        let area = if main_chunks.len() > 1 {
            main_chunks[1]
        } else {
            outer[0]
        };
        app.preview_area_height = area.height;
        f.render_widget(preview, area);
    }

    let help_text = if let Some(ref msg) = app.status_message {
        msg.clone()
    } else {
        let base =
            "q:quit • ?:help • Enter/o:open • m:merge • r:reload • p:toggle • b:body • d:diff";
        let nav = if app.preview_open {
            // Implemented: j/k, ↑/↓ (scroll); mouse wheel scrolls faster
            "j/k or ↑/↓:scroll • wheel:scroll"
        } else {
            // Implemented: j/k, ↑/↓ navigation in list
            "j/k or ↑/↓:navigate"
        };
        let mode = match app.preview_mode {
            PreviewMode::Body => "Body",
            PreviewMode::Diff => "Diff",
        };
        format!("{} • {} • mode:{}", base, nav, mode)
    };

    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .wrap(Wrap { trim: true });

    f.render_widget(help, outer[1]);
}

async fn fetch_pr_body(owner: &str, name: &str, number: usize) -> surf::Result<String> {
    let vars = json!({ "owner": owner, "name": name, "number": number as i64 });
    let q = json!({ "query": include_str!("../query/pr.body.graphql"), "variables": vars });
    let res = crate::graphql::query::<pr_body_res::PrBodyRes>(&q).await?;
    Ok(res.data.repository.pull_request.body_text)
}

#[derive(Deserialize)]
struct PrFileRes {
    filename: String,
    additions: i64,
    deletions: i64,
    patch: Option<String>,
}

struct PrFile {
    filename: String,
    additions: i64,
    deletions: i64,
    patch: Option<String>,
}

async fn fetch_pr_files(owner: &str, name: &str, number: usize) -> surf::Result<Vec<PrFile>> {
    let path = format!("repos/{}/{}/pulls/{}/files", owner, name, number);
    let q: rest::QueryMap = Default::default();
    let res: Vec<PrFileRes> = rest::get(&path, 1, &q).await?;
    Ok(res
        .into_iter()
        .map(|f| PrFile {
            filename: f.filename,
            additions: f.additions,
            deletions: f.deletions,
            patch: f.patch,
        })
        .collect())
}
