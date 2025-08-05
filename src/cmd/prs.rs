use colored::Colorize;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
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
        let mut all_prs = Vec::new();
        for slug in slugs {
            let vs: Vec<String> = slug.split('/').map(String::from).collect();
            match vs.len() {
                1 => {
                    let prs = fetch_owner_prs(&vs[0]).await?;
                    all_prs.extend(prs);
                }
                2 => {
                    let prs = fetch_repo_prs(&vs[0], &vs[1]).await?;
                    all_prs.extend(prs);
                }
                _ => panic!("unknown slug format"),
            }
        }
        run_tui(all_prs).map_err(|e| {
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
            String::new()
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

struct App {
    prs: Vec<PrData>,
    list_state: ListState,
    should_quit: bool,
    status_message: Option<String>,
}

impl App {
    fn new(prs: Vec<PrData>) -> App {
        let mut list_state = ListState::default();
        if !prs.is_empty() {
            list_state.select(Some(0));
        }
        App {
            prs,
            list_state,
            should_quit: false,
            status_message: None,
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
                    self.status_message = Some(format!("Merging PR #{}...", pr.number));
                    match merge_pr(&pr.id).await {
                        Ok(_) => {
                            self.status_message = Some(format!("✅ Merged PR #{}", pr.number));
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
                            self.status_message =
                                Some(format!("❌ Failed to merge PR #{}: {}", pr.number, e));
                        }
                    }
                } else {
                    self.status_message = Some(format!(
                        "Cannot merge PR #{}: not in clean state",
                        pr.number
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
}

fn run_tui(prs: Vec<PrData>) -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(prs);
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
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => {
                        app.should_quit = true;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.next();
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.previous();
                    }
                    KeyCode::Enter | KeyCode::Char('o') => {
                        app.open_url();
                    }
                    KeyCode::Char('m') => {
                        app.merge_selected().await;
                    }
                    _ => {}
                }
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)].as_ref())
        .split(f.area());

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

    f.render_stateful_widget(items, chunks[0], &mut app.list_state);

    let help_text = if let Some(ref msg) = app.status_message {
        msg.clone()
    } else {
        "Press 'q' to quit, 'j/k' or ↑/↓ to navigate, 'Enter' or 'o' to open in browser, 'm' to merge (if clean)".to_string()
    };

    let help = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .wrap(Wrap { trim: true });

    f.render_widget(help, chunks[1]);
}
