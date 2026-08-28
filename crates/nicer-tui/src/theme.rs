//! The palette. Named, not scattered: every color used anywhere in the UI is one of these.
//!
//! `nice`/`wire`/`cut`/`signal` double as both a visual accent and a semantic tag (accepted,
//! pending, rejected, incoming) so color carries meaning instead of decorating.

use ratatui::style::Color;

pub const VOID: Color = Color::Rgb(0x0b, 0x0e, 0x14);
pub const PANEL: Color = Color::Rgb(0x12, 0x18, 0x22);
pub const PANEL_ALT: Color = Color::Rgb(0x1a, 0x22, 0x2e);
pub const RAIL: Color = Color::Rgb(0x2a, 0x33, 0x42);

pub const BONE: Color = Color::Rgb(0xe8, 0xec, 0xf1);
pub const SLATE: Color = Color::Rgb(0x6b, 0x76, 0x87);
pub const DIM: Color = Color::Rgb(0x46, 0x4f, 0x5c);

/// secure · paired · accepted · verified clean
pub const NICE: Color = Color::Rgb(0x7c, 0xe3, 0x8b);
/// pending · awaiting a human · big_diff
pub const WIRE: Color = Color::Rgb(0xe8, 0xb3, 0x39);
/// rejected · failed · corrupt · fatal
pub const CUT: Color = Color::Rgb(0xf2, 0x55, 0x5a);
/// discovery · incoming · chat
pub const SIGNAL: Color = Color::Rgb(0x5f, 0xa8, 0xff);
