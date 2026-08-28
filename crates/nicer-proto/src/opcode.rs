use std::fmt;

/// Which stream an opcode is permitted on. See RFC R3.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    Connection,
    Stream,
    Either,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum Opcode {
    Hello = 0x01,
    Merged = 0x02,
    Tux = 0x03,
    Subsurface = 0x04,
    PullRequest = 0x10,
    Merge = 0x11,
    FuckOff = 0x12,
    ShutUp = 0x13,
    Diff = 0x20,
    Done = 0x21,
    Fsck = 0x22,
    Clean = 0x23,
    Corrupt = 0x24,
    BigDiff = 0x25,
    Lkml = 0x30,
    Bitkeeper = 0x40,
    Git = 0x41,
    Monotone = 0x42,
    Cpp = 0x7C,
    BrokeUserspace = 0x7D,
    Nvidia = 0x7E,
}

impl Opcode {
    pub const ALL: [Opcode; 21] = [
        Opcode::Hello,
        Opcode::Merged,
        Opcode::Tux,
        Opcode::Subsurface,
        Opcode::PullRequest,
        Opcode::Merge,
        Opcode::FuckOff,
        Opcode::ShutUp,
        Opcode::Diff,
        Opcode::Done,
        Opcode::Fsck,
        Opcode::Clean,
        Opcode::Corrupt,
        Opcode::BigDiff,
        Opcode::Lkml,
        Opcode::Bitkeeper,
        Opcode::Git,
        Opcode::Monotone,
        Opcode::Cpp,
        Opcode::BrokeUserspace,
        Opcode::Nvidia,
    ];

    pub fn from_u8(byte: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|op| *op as u8 == byte)
    }

    pub fn name(self) -> &'static str {
        match self {
            Opcode::Hello => "HELLO",
            Opcode::Merged => "MERGED",
            Opcode::Tux => "TUX",
            Opcode::Subsurface => "SUBSURFACE",
            Opcode::PullRequest => "PULL_REQUEST",
            Opcode::Merge => "MERGE",
            Opcode::FuckOff => "FUCK_OFF",
            Opcode::ShutUp => "SHUT_UP",
            Opcode::Diff => "DIFF",
            Opcode::Done => "DONE",
            Opcode::Fsck => "FSCK",
            Opcode::Clean => "CLEAN",
            Opcode::Corrupt => "CORRUPT",
            Opcode::BigDiff => "BIG_DIFF",
            Opcode::Lkml => "LKML",
            Opcode::Bitkeeper => "BITKEEPER",
            Opcode::Git => "GIT",
            Opcode::Monotone => "MONOTONE",
            Opcode::Cpp => "CPP",
            Opcode::BrokeUserspace => "BROKE_USERSPACE",
            Opcode::Nvidia => "NVIDIA",
        }
    }

    pub fn scope(self) -> Scope {
        match self {
            Opcode::Hello
            | Opcode::Merged
            | Opcode::Tux
            | Opcode::Subsurface
            | Opcode::Bitkeeper
            | Opcode::Git
            | Opcode::Monotone
            | Opcode::Nvidia => Scope::Connection,
            Opcode::PullRequest
            | Opcode::Merge
            | Opcode::FuckOff
            | Opcode::BigDiff
            | Opcode::Diff
            | Opcode::Done
            | Opcode::Fsck
            | Opcode::Clean
            | Opcode::Corrupt
            | Opcode::Lkml => Scope::Stream,
            Opcode::Cpp | Opcode::ShutUp | Opcode::BrokeUserspace => Scope::Either,
        }
    }

    pub fn is_fatal_error(self) -> bool {
        matches!(self, Opcode::BrokeUserspace | Opcode::Nvidia)
    }

    pub fn is_error(self) -> bool {
        matches!(
            self,
            Opcode::Cpp | Opcode::BrokeUserspace | Opcode::Nvidia | Opcode::Monotone
        )
    }

    pub fn opens_stream(self) -> bool {
        matches!(self, Opcode::PullRequest | Opcode::Lkml)
    }
}

impl fmt::Display for Opcode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
