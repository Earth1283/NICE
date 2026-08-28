use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState};
use ratatui::Frame;

use crate::app::{App, PeersPane};
use crate::theme;
use crate::ui::block;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::new(Direction::Horizontal, [Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    draw_paired(frame, app, cols[0]);
    draw_discovered(frame, app, cols[1]);
}

fn tag(on: bool, label: &str) -> Span<'static> {
    if on {
        Span::styled(format!(" {label} "), Style::new().fg(theme::VOID).bg(theme::NICE))
    } else {
        Span::styled(format!(" {label} "), Style::new().fg(theme::SLATE))
    }
}

fn draw_paired(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.peers_pane == PeersPane::Paired;
    let items: Vec<ListItem> = if app.peers.is_empty() {
        vec![ListItem::new(Span::styled(
            "no paired peers yet — pair from the Connections tab",
            Style::new().fg(theme::DIM),
        ))]
    } else {
        app.peers
            .iter()
            .map(|p| {
                let addr = p
                    .last_address
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "unknown address".into());
                ListItem::new(vec![Line::from(vec![
                    Span::styled(format!("{:<20}", p.device), Style::new().fg(theme::BONE).bold()),
                    Span::styled(format!(" {}  ", p.fingerprint.short()), Style::new().fg(theme::SLATE)),
                    tag(p.auto_accept_files, "F"),
                    Span::raw(" "),
                    tag(p.auto_accept_clipboard, "C"),
                    Span::styled(format!("  {addr}"), Style::new().fg(theme::SLATE)),
                ])])
            })
            .collect()
    };

    let mut state = ListState::default();
    if !app.peers.is_empty() {
        state.select(Some(app.peers_index));
    }
    let list = List::new(items)
        .block(block("Paired  (F)ile  (C)lipboard auto-accept", focused))
        .highlight_style(Style::new().bg(theme::PANEL_ALT));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_discovered(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.peers_pane == PeersPane::Discovered;
    let items: Vec<ListItem> = if app.discovered.is_empty() {
        vec![ListItem::new(Span::styled(
            "listening for mDNS…",
            Style::new().fg(theme::DIM),
        ))]
    } else {
        app.discovered
            .values()
            .map(|d| {
                let addr = d
                    .addresses
                    .first()
                    .map(|a| format!("{a}:{}", d.port))
                    .unwrap_or_default();
                let secure = if d.secure {
                    Span::styled(" SECURE ", Style::new().fg(theme::VOID).bg(theme::NICE))
                } else {
                    Span::styled(" PLAINTEXT ", Style::new().fg(theme::VOID).bg(theme::CUT))
                };
                let fingerprint = d
                    .fingerprint
                    .map(|f| format!("  {}", f.short()))
                    .unwrap_or_default();
                ListItem::new(vec![Line::from(vec![
                    Span::styled(format!("{:<18}", d.device), Style::new().fg(theme::BONE).bold()),
                    secure,
                    Span::styled(format!(" {addr}"), Style::new().fg(theme::SLATE)),
                    Span::styled(fingerprint, Style::new().fg(theme::DIM)),
                ])])
            })
            .collect()
    };

    let mut state = ListState::default();
    if !app.discovered.is_empty() {
        state.select(Some(app.discovered_index));
    }
    let list = List::new(items)
        .block(block("Discovered — never trusted, Enter to dial", focused))
        .highlight_style(Style::new().bg(theme::PANEL_ALT));
    frame.render_stateful_widget(list, area, &mut state);
}
