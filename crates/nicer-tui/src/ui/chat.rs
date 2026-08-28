use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::theme;
use crate::ui::block;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let rows = Layout::new(Direction::Vertical, [Constraint::Length(1), Constraint::Min(1)]).split(area);

    let peer_line = match app.chat_connection {
        Some(id) => Line::from(vec![
            Span::styled(" LKML ", Style::new().fg(theme::VOID).bg(theme::SIGNAL)),
            Span::raw(format!(" {}  (h/l to switch)", app.connection_label(id))),
        ]),
        None => Line::from(Span::styled(
            " no connection selected — open one from Connections",
            Style::new().fg(theme::DIM),
        )),
    };
    frame.render_widget(Paragraph::new(peer_line), rows[0]);

    let b = block("Chat", true);
    let inner = b.inner(rows[1]);
    frame.render_widget(b, rows[1]);

    let lines: Vec<Line> = match app.chat_connection.and_then(|id| app.chats.get(&id)) {
        Some(chat) if !chat.is_empty() => chat
            .iter()
            .map(|line| {
                let who = if line.from_us { "you".to_string() } else { line.from.clone() };
                // No identity was proven for a PLAINTEXT connection; flag it in the color
                // itself rather than an extra glyph column, since emoji width is unreliable
                // across terminals.
                let color = match (line.from_us, line.fingerprint.is_some()) {
                    (true, _) => theme::NICE,
                    (false, true) => theme::SIGNAL,
                    (false, false) => theme::CUT,
                };
                Line::from(vec![
                    Span::styled(format!("[{who}] "), Style::new().fg(color).bold()),
                    Span::styled(line.text.clone(), Style::new().fg(theme::BONE)),
                ])
            })
            .collect(),
        _ => vec![Line::from(Span::styled(
            "nothing yet — press 'i' to send a line",
            Style::new().fg(theme::DIM),
        ))],
    };

    let height = inner.height as usize;
    let total = lines.len();
    let from_bottom = app.chat_scroll as usize;
    let skip = total.saturating_sub(height).saturating_sub(from_bottom.min(total));

    frame.render_widget(
        Paragraph::new(lines).scroll((skip as u16, 0)).style(Style::new().bg(theme::PANEL)),
        inner,
    );
}
