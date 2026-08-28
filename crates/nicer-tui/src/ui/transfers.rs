use nicer::event::Direction;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState};
use ratatui::Frame;

use crate::app::{offer_kind_label, App, TransferStatus};
use crate::theme;
use crate::ui::block;

fn dir_label(direction: Direction) -> &'static str {
    match direction {
        Direction::Incoming => "IN ",
        Direction::Outgoing => "OUT",
    }
}

fn bar(transferred: u64, total: u64, width: usize) -> String {
    if total == 0 || width == 0 {
        return String::new();
    }
    let filled = ((transferred as f64 / total as f64) * width as f64).round() as usize;
    let filled = filled.min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}B")
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let entries = app.ordered_transfers();
    let items: Vec<ListItem> = if entries.is_empty() {
        vec![ListItem::new(Span::styled(
            "nothing offered, sent, or received this session",
            Style::new().fg(theme::DIM),
        ))]
    } else {
        entries
            .iter()
            .map(|e| {
                let who = app.connection_label(e.connection);
                let name = e.name.clone().unwrap_or_else(|| "-".into());
                let head = Line::from(vec![
                    Span::styled(format!("{who:<24} "), Style::new().fg(theme::SLATE)),
                    Span::styled(name.to_string(), Style::new().fg(theme::BONE).bold()),
                ]);
                let tail = match &e.status {
                    TransferStatus::Offered { kind, size, .. } => Line::from(vec![
                        Span::styled(" AWAITING RESPONSE ", Style::new().fg(theme::VOID).bg(theme::WIRE)),
                        Span::raw(format!(" {} {}", offer_kind_label(*kind), human(*size))),
                    ]),
                    TransferStatus::Accepted { kind } => Line::from(vec![
                        Span::styled(" MERGED ", Style::new().fg(theme::VOID).bg(theme::NICE)),
                        Span::raw(format!(" {} — starting…", offer_kind_label(*kind))),
                    ]),
                    TransferStatus::Active { direction, transferred, total, .. } => Line::from(vec![
                        Span::styled(format!("{} ", dir_label(*direction)), Style::new().fg(theme::SIGNAL)),
                        Span::styled(bar(*transferred, *total, 24), Style::new().fg(theme::SIGNAL)),
                        Span::raw(format!(" {}/{}", human(*transferred), human(*total))),
                    ]),
                    TransferStatus::Complete { direction, path, .. } => Line::from(vec![
                        Span::styled(" DONE ", Style::new().fg(theme::VOID).bg(theme::NICE)),
                        Span::raw(format!(" {} ", dir_label(*direction))),
                        Span::styled(
                            path.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                            Style::new().fg(theme::SLATE),
                        ),
                    ]),
                    TransferStatus::Failed { direction, reason } => Line::from(vec![
                        Span::styled(format!("{} ", dir_label(*direction)), Style::new().fg(theme::SLATE)),
                        Span::styled(" FAILED ", Style::new().fg(theme::VOID).bg(theme::CUT)),
                        Span::raw(format!(" {reason}")),
                    ]),
                    TransferStatus::Refused { direction, reason } => Line::from(vec![
                        Span::styled(format!("{} ", dir_label(*direction)), Style::new().fg(theme::SLATE)),
                        Span::styled(" FUCK_OFF ", Style::new().fg(theme::VOID).bg(theme::CUT)),
                        Span::raw(format!(" {}", reason.as_deref().unwrap_or("no reason given"))),
                    ]),
                    TransferStatus::TooBig { direction, max_size } => Line::from(vec![
                        Span::styled(format!("{} ", dir_label(*direction)), Style::new().fg(theme::SLATE)),
                        Span::styled(" BIG_DIFF ", Style::new().fg(theme::VOID).bg(theme::WIRE)),
                        Span::raw(format!(
                            " wants <= {}",
                            max_size.map(human).unwrap_or_else(|| "?".into())
                        )),
                    ]),
                };
                ListItem::new(vec![head, tail])
            })
            .collect()
    };

    let mut state = ListState::default();
    if !entries.is_empty() {
        state.select(Some(app.transfers_index));
    }
    let list = List::new(items)
        .block(block("Transfers — offers, sends, receipts", true))
        .highlight_style(Style::new().bg(theme::PANEL_ALT));
    frame.render_stateful_widget(list, area, &mut state);
}
