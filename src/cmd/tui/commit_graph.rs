use crate::cmd::prs::{Commit, CommitGraphEntry};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};
use std::collections::HashSet;

pub(super) fn build_commit_graph_entries(commits: &[Commit]) -> Vec<CommitGraphEntry> {
    let mut active: Vec<String> = Vec::new();
    let mut lines: Vec<CommitGraphEntry> = Vec::new();

    for commit in commits.iter().rev() {
        // Ensure the current commit is the first active branch.
        if let Some(pos) = active.iter().position(|sha| sha == &commit.sha) {
            let sha = active.remove(pos);
            active.insert(0, sha);
        } else {
            active.insert(0, commit.sha.clone());
        }

        lines.push(CommitGraphEntry {
            graph: build_graph_prefix(&active),
            short_sha: commit.sha.chars().take(7).collect::<String>(),
            summary: commit.summary(),
            author: commit.display_author(),
            date: commit.display_date(),
        });

        // Remove the commit itself and add parents to track branch lines.
        active.remove(0);
        for (idx, parent) in commit.parent_shas().enumerate() {
            if let Some(existing) = active.iter().position(|sha| sha == parent) {
                let sha = active.remove(existing);
                active.insert(idx, sha);
            } else {
                active.insert(idx, parent.to_string());
            }
        }
        dedup_branches(&mut active);
    }

    lines
}

fn build_graph_prefix(active: &[String]) -> String {
    let mut prefix = String::default();
    for (idx, _) in active.iter().enumerate() {
        if idx == 0 {
            prefix.push('*');
        } else {
            prefix.push('|');
        }
        prefix.push(' ');
    }
    prefix
}

fn dedup_branches(branches: &mut Vec<String>) {
    let mut seen: HashSet<String> = HashSet::new();
    branches.retain(|sha| seen.insert(sha.clone()));
}

pub(super) fn make_commit_graph_text(entries: &[CommitGraphEntry]) -> Text<'static> {
    if entries.is_empty() {
        return Text::from("No commits found.");
    }

    let mut text = Text::default();
    for entry in entries {
        let mut spans: Vec<Span> = Vec::new();
        if !entry.graph.is_empty() {
            spans.push(Span::styled(
                entry.graph.clone(),
                Style::default().fg(Color::DarkGray),
            ));
        }
        spans.push(Span::styled(
            entry.short_sha.clone(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::raw(entry.summary.clone()));
        if let Some(author) = &entry.author {
            spans.push(Span::raw("  • "));
            spans.push(Span::styled(
                author.clone(),
                Style::default().fg(Color::Cyan),
            ));
        }
        if let Some(date) = &entry.date {
            spans.push(Span::raw("  ("));
            spans.push(Span::styled(date.clone(), Style::default().fg(Color::Gray)));
            spans.push(Span::raw(")"));
        }
        text.lines.push(Line::from(spans));
    }
    text
}
