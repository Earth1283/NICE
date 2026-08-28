use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{offer_kind_label, App, Modal, TransferStatus};
use crate::theme;

fn centered(width: u16, height: u16, area: Rect) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    area
}

fn frame_box(title: &str, color: ratatui::style::Color) -> Block<'static> {
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::new().fg(color))
        .style(Style::new().bg(theme::PANEL_ALT))
        .title(Span::styled(format!(" {title} "), Style::new().fg(theme::BONE).bold()))
}

fn draw_box(frame: &mut Frame, area: Rect, width: u16, height: u16, title: &str, color: ratatui::style::Color, lines: Vec<Line<'static>>) {
    let rect = centered(width, height, area);
    frame.render_widget(Clear, rect);
    let block = frame_box(title, color);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn input_box(frame: &mut Frame, area: Rect, title: &str, prompt: &str, input: &str) {
    let lines = vec![
        Line::from(Span::styled(prompt.to_string(), Style::new().fg(theme::SLATE))),
        Line::from(""),
        Line::from(vec![
            Span::styled("> ", Style::new().fg(theme::NICE)),
            Span::styled(input.to_string(), Style::new().fg(theme::BONE)),
            Span::styled("_", Style::new().fg(theme::NICE)),
        ]),
        Line::from(""),
        Line::from(Span::styled("Enter to send · Esc to cancel", Style::new().fg(theme::DIM))),
    ];
    draw_box(frame, area, 60, 7, title, theme::NICE, lines);
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    match &app.modal {
        Modal::None => {}
        Modal::Help => draw_help(frame, area),
        Modal::Connect => input_box(frame, area, "Connect", "address to dial, host:port", &app.input),
        Modal::SendFile { .. } => input_box(
            frame,
            area,
            "Send file",
            "absolute path on the daemon's own filesystem",
            &app.input,
        ),
        Modal::SendClipboardText { .. } => {
            input_box(frame, area, "Send clipboard text", "text to offer as clipboard", &app.input)
        }
        Modal::ChatInput { connection } => input_box(
            frame,
            area,
            "LKML",
            &format!("message to {}", app.connection_label(*connection)),
            &app.input,
        ),
        Modal::BigDiffSize { .. } => input_box(
            frame,
            area,
            "BIG_DIFF",
            "max size in bytes you'd accept (blank = no hint)",
            &app.input,
        ),
        Modal::FuckOffReason { .. } => {
            input_box(frame, area, "FUCK_OFF", "reason (optional, shown to the sender)", &app.input)
        }
        Modal::ConfirmUnpair { device, fingerprint } => {
            let lines = vec![
                Line::from(format!("Unpair {device}?")),
                Line::from(Span::styled(fingerprint.short(), Style::new().fg(theme::SLATE))),
                Line::from(""),
                Line::from("This revokes trust. Auto-accept for this peer goes with it."),
                Line::from(""),
                Line::from(Span::styled("y confirm · n/Esc cancel", Style::new().fg(theme::DIM))),
            ];
            draw_box(frame, area, 56, 8, "Unpair", theme::CUT, lines);
        }
        Modal::Pairing(prompt) => {
            let lines = vec![
                Line::from(vec![
                    Span::raw("device      "),
                    Span::styled(prompt.device.clone(), Style::new().fg(theme::BONE).bold()),
                ]),
                Line::from(format!("address     {}", prompt.address)),
                Line::from(""),
                Line::from(Span::styled("compare this against their screen:", Style::new().fg(theme::SLATE))),
                Line::from(Span::styled(
                    prompt.short.clone(),
                    Style::new().fg(theme::WIRE).bold(),
                )),
                Line::from(Span::styled(prompt.fingerprint.to_string(), Style::new().fg(theme::DIM))),
                Line::from(""),
                Line::from(Span::styled("y pair · n/Esc postpone (reopen with 'p')", Style::new().fg(theme::DIM))),
            ];
            draw_box(frame, area, 64, 11, "Pairing required", theme::WIRE, lines);
        }
        Modal::Offer { connection, stream } => {
            let entry = app
                .transfers
                .iter()
                .find(|e| e.connection == *connection && e.stream == *stream);
            let mut lines = vec![Line::from(format!("from {}", app.connection_label(*connection)))];
            if let Some(entry) = entry {
                if let TransferStatus::Offered { kind, size, mime, preview } = &entry.status {
                    lines.push(Line::from(vec![
                        Span::styled(offer_kind_label(*kind), Style::new().fg(theme::SIGNAL).bold()),
                        Span::raw(format!("  {size} bytes")),
                    ]));
                    if let Some(name) = &entry.name {
                        lines.push(Line::from(format!("name  {name}")));
                    }
                    if let Some(mime) = mime {
                        lines.push(Line::from(format!("mime  {mime}")));
                    }
                    if let Some(preview) = preview {
                        lines.push(Line::from(""));
                        lines.push(Line::from(Span::styled(preview.clone(), Style::new().fg(theme::BONE))));
                    }
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "m merge · f fuck off · F fuck off w/ reason · b big_diff · Esc postpone",
                Style::new().fg(theme::DIM),
            )));
            draw_box(frame, area, 66, lines.len() as u16 + 2, "Incoming PULL_REQUEST", theme::SIGNAL, lines);
        }
    }
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(Span::styled("global", Style::new().fg(theme::WIRE).bold())),
        Line::from("  q / ^C quit    Tab/S-Tab or 1-5 switch tab    n new connection    Esc close"),
        Line::from(""),
        Line::from(Span::styled("peers", Style::new().fg(theme::WIRE).bold())),
        Line::from("  h/l pane   j/k move   Enter connect   u unpair   f/c toggle file/clipboard auto-accept"),
        Line::from(""),
        Line::from(Span::styled("connections", Style::new().fg(theme::WIRE).bold())),
        Line::from("  j/k move   p pair   d disconnect   s resync (BITKEEPER/GIT)   f send file"),
        Line::from("  v send own clipboard   V send custom text   g open in Chat"),
        Line::from(""),
        Line::from(Span::styled("transfers / chat / log", Style::new().fg(theme::WIRE).bold())),
        Line::from("  j/k move or scroll   Enter on a pending offer to answer it   i to write a chat line"),
        Line::from(""),
        Line::from(Span::styled("the wire, §7", Style::new().fg(theme::WIRE).bold())),
        Line::from("  HELLO/MERGED handshake · PULL_REQUEST/MERGE/FUCK_OFF/BIG_DIFF offers"),
        Line::from("  DIFF/DONE/FSCK/CLEAN/CORRUPT transfer+integrity · LKML chat · SHUT_UP rate limit"),
        Line::from("  BITKEEPER/GIT/MONOTONE recovery · CPP/BROKE_USERSPACE/NVIDIA errors"),
        Line::from(""),
        Line::from(Span::styled("any key closes this", Style::new().fg(theme::DIM))),
    ];
    draw_box(frame, area, 78, lines.len() as u16 + 2, "Help", theme::SLATE, lines);
}
