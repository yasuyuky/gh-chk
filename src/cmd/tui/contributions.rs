use ratatui::text::Line;

pub(super) struct ContributionView {
    pub(super) lines: Option<Vec<Line<'static>>>,
    pub(super) stats: Option<Vec<Line<'static>>>,
    pub(super) height: u16,
    pub(super) title: String,
}

impl Default for ContributionView {
    fn default() -> Self {
        Self {
            lines: None,
            stats: None,
            height: 9,
            title: "Contributions".to_string(),
        }
    }
}
