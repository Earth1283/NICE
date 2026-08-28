use nicer::event::Direction;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState};
use ratatui::Frame;

use crate::app::App;
use crate::theme;
use crate::ui::block;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = if app.connections.is_empty() {
        vec![ListItem::new(Span::styled(
            "nothing open — 'n' to dial an address, or connect from Peers",
            Style::new().fg(theme::DIM),
        ))]
    } else {
        app.connections
            .values()
            .map(|c| {
                let arrow = match c.direction {
                    Direction::Outgoing => Span::styled("-> ", Style::new().fg(theme::SIGNAL)),
                    Direction::Incoming => Span::styled("<- ", Style::new().fg(theme::NICE)),
                };
                let paired = if c.paired {
                    Span::styled(" PAIRED ", Style::new().fg(theme::VOID).bg(theme::NICE))
                } else if c.fingerprint.is_some() {
                    Span::styled(" UNPAIRED ", Style::new().fg(theme::VOID).bg(theme::WIRE))
                } else {
                    Span::styled(" PLAINTEXT ", Style::new().fg(theme::VOID).bg(theme::CUT))
                };
                ListItem::new(vec![Line::from(vec![
                    Span::styled(format!("#{:<4}", c.connection), Style::new().fg(theme::SLATE)),
                    arrow,
                    Span::styled(format!("{:<16}", c.device), Style::new().fg(theme::BONE).bold()),
                    Span::styled(format!("{:<21}", c.address.to_string()), Style::new().fg(theme::SLATE)),
                    paired,
                ])])
            })
            .collect()
    };

    let mut state = ListState::default();
    if !app.connections.is_empty() {
        state.select(Some(app.connections_index));
    }
    let list = List::new(items)
        .block(block("Connections", true))
        .highlight_style(Style::new().bg(theme::PANEL_ALT));
    frame.render_stateful_widget(list, area, &mut state);
}
