//! The signature element: NICE/1 boasts "intentionally excellent Wireshark visibility", so
//! the header renders a live strip of the opcodes each [`Event`] implies, grounded in
//! `NICE-1.md` §7-16 rather than invented. `protocol_error` is the one honest case — the
//! daemon hands back the literal opcode name that crossed the wire.

use nicer::event::{Direction, Event, Verdict};
use ratatui::style::Color;

use crate::theme;

pub struct Tick {
    pub opcode: String,
    pub detail: String,
    pub color: Color,
}

impl Tick {
    fn new(opcode: &str, detail: impl Into<String>, color: Color) -> Self {
        Tick {
            opcode: opcode.to_string(),
            detail: detail.into(),
            color,
        }
    }
}

/// `None` means the event has no wire opcode worth surfacing (mDNS chatter, local judgement
/// calls like pairing, or a startup notice) — those still land in the log tab.
pub fn tick_for(event: &Event) -> Option<Tick> {
    match event {
        Event::Connected { direction, .. } => Some(match direction {
            Direction::Outgoing => Tick::new("HELLO", "we dialled", theme::SIGNAL),
            Direction::Incoming => Tick::new("MERGED", "we accepted", theme::NICE),
        }),
        Event::Disconnected { reason, .. } => Some(Tick::new("·", reason.clone(), theme::SLATE)),
        Event::Offer {
            kind, name, size, ..
        } => Some(Tick::new(
            "PULL_REQUEST",
            format!("{kind:?} {} ({size}B)", name.as_deref().unwrap_or("-")),
            theme::SIGNAL,
        )),
        Event::OfferResolved { verdict, .. } => Some(match verdict {
            Verdict::Merge => Tick::new("MERGE", "accepted", theme::NICE),
            Verdict::FuckOff { reason } => Tick::new(
                "FUCK_OFF",
                reason.clone().unwrap_or_else(|| "no reason given".into()),
                theme::CUT,
            ),
            Verdict::BigDiff { max_size } => Tick::new(
                "BIG_DIFF",
                match max_size {
                    Some(n) => format!("wants <= {n}B"),
                    None => "too big".into(),
                },
                theme::WIRE,
            ),
        }),
        Event::TransferProgress {
            transferred, total, ..
        } => Some(Tick::new(
            "DIFF",
            format!("{transferred}/{total}"),
            theme::SIGNAL,
        )),
        Event::TransferComplete { .. } => Some(Tick::new("DONE", "verified clean", theme::NICE)),
        Event::TransferFailed { reason, .. } => {
            Some(Tick::new("CORRUPT", reason.clone(), theme::CUT))
        }
        Event::Chat { text, .. } => Some(Tick::new("LKML", text.clone(), theme::BONE)),
        Event::RateLimited {
            retry_after_ms,
            scope,
            ..
        } => Some(Tick::new(
            "SHUT_UP",
            format!("{scope:?} for {retry_after_ms}ms"),
            theme::WIRE,
        )),
        Event::ProtocolError { opcode, reason, .. } => {
            Some(Tick::new(opcode, reason.clone(), theme::CUT))
        }
        Event::Resynced { .. } => Some(Tick::new("GIT", "fresh sync point", theme::WIRE)),
        Event::Listening { .. }
        | Event::PeerDiscovered { .. }
        | Event::PeerLost { .. }
        | Event::PairingRequired { .. }
        | Event::IdentityChanged { .. }
        | Event::Warning { .. } => None,
    }
}
