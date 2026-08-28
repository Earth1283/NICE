use std::fmt;

/// Stream 0, reserved for connection-level messages.
pub const CONNECTION: StreamId = StreamId(0);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct StreamId(pub u32);

impl StreamId {
    pub fn is_connection(self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for StreamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Which half of the connection a peer is, fixed by who sent `HELLO`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Initiator,
    Responder,
}

impl Role {
    pub fn peer(self) -> Role {
        match self {
            Role::Initiator => Role::Responder,
            Role::Responder => Role::Initiator,
        }
    }

    /// RFC R3: initiator streams are odd, responder streams are even and nonzero.
    pub fn owns(self, id: StreamId) -> bool {
        match self {
            _ if id.is_connection() => false,
            Role::Initiator => id.0 % 2 == 1,
            Role::Responder => id.0 % 2 == 0,
        }
    }

    fn first(self) -> u32 {
        match self {
            Role::Initiator => 1,
            Role::Responder => 2,
        }
    }
}

/// Hands out locally opened stream identifiers. Never wraps (RFC R3).
#[derive(Debug)]
pub struct StreamAllocator {
    role: Role,
    next: u32,
}

impl StreamAllocator {
    pub fn new(role: Role) -> Self {
        Self {
            role,
            next: role.first(),
        }
    }

    pub fn allocate(&mut self) -> Option<StreamId> {
        let id = StreamId(self.next);
        self.next = self.next.checked_add(2)?;
        Some(id)
    }

    pub fn reset(&mut self) {
        self.next = self.role.first();
    }

    pub fn role(&self) -> Role {
        self.role
    }
}

/// Rejects peer-opened streams that repeat or move backwards (RFC R3).
#[derive(Debug)]
pub struct PeerStreams {
    peer: Role,
    highest: u32,
}

impl PeerStreams {
    pub fn new(local: Role) -> Self {
        Self {
            peer: local.peer(),
            highest: 0,
        }
    }

    pub fn accept_new(&mut self, id: StreamId) -> Result<(), StreamViolation> {
        if !self.peer.owns(id) {
            return Err(StreamViolation::WrongParity(id));
        }
        if id.0 <= self.highest {
            return Err(StreamViolation::NotIncreasing(id));
        }
        self.highest = id.0;
        Ok(())
    }

    pub fn is_stale(&self, id: StreamId) -> bool {
        self.peer.owns(id) && id.0 <= self.highest
    }

    pub fn reset(&mut self) {
        self.highest = 0;
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StreamViolation {
    WrongParity(StreamId),
    NotIncreasing(StreamId),
}

impl fmt::Display for StreamViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamViolation::WrongParity(id) => {
                write!(f, "stream {id} has the wrong parity for its opener")
            }
            StreamViolation::NotIncreasing(id) => {
                write!(
                    f,
                    "stream {id} reuses or precedes an identifier already seen"
                )
            }
        }
    }
}
