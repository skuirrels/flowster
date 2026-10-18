//! `flowster-sync-proto`: frames exchanged over `wss` (SPEC §20.8).
//!
//! ```text
//! frame   = magic "FS" · version u8 · tag u8 · body
//! body    = fields in declaration order
//! bytes   = len varint (LEB128) · raw
//! string  = bytes (UTF-8)
//! u64/u16 = varint
//! ```
//!
//! CRDT payloads (`Update`, `SyncUpdates`, version vectors, `Ephemeral`) are
//! opaque bytes produced by the CRDT library, so this crate does not depend on it.

#![forbid(unsafe_code)]

/// Wire version. The server rejects clients below [`MIN_SUPPORTED_VERSION`]
/// with [`ErrorCode::ClientTooOld`] so the app can prompt for an upgrade.
pub const PROTOCOL_VERSION: u8 = 1;
pub const MIN_SUPPORTED_VERSION: u8 = 1;
const MAGIC: [u8; 2] = *b"FS";
/// Hard cap on a decoded frame; larger documents go through snapshots and blobs.
pub const MAX_FRAME_BYTES: usize = 64 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Owner,
    Editor,
    Commenter,
    Viewer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    ClientTooOld,
    Unauthorized,
    Forbidden,
    NotFound,
    InvalidUpdate,
    TooLarge,
    RateLimited,
    Internal,
    Unknown(u16),
}

impl ErrorCode {
    fn to_u16(self) -> u16 {
        match self {
            ErrorCode::ClientTooOld => 1,
            ErrorCode::Unauthorized => 2,
            ErrorCode::Forbidden => 3,
            ErrorCode::NotFound => 4,
            ErrorCode::InvalidUpdate => 5,
            ErrorCode::TooLarge => 6,
            ErrorCode::RateLimited => 7,
            ErrorCode::Internal => 8,
            ErrorCode::Unknown(c) => c,
        }
    }
    fn from_u16(c: u16) -> Self {
        match c {
            1 => ErrorCode::ClientTooOld,
            2 => ErrorCode::Unauthorized,
            3 => ErrorCode::Forbidden,
            4 => ErrorCode::NotFound,
            5 => ErrorCode::InvalidUpdate,
            6 => ErrorCode::TooLarge,
            7 => ErrorCode::RateLimited,
            8 => ErrorCode::Internal,
            c => ErrorCode::Unknown(c),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    /// Client → server, first frame on a socket.
    Hello {
        doc_id: String,
        token: String,
        client_version_vector: Vec<u8>,
    },
    /// Server → client reply to `Hello`.
    Welcome {
        role: Role,
        server_version_vector: Vec<u8>,
    },
    /// Either way, after the handshake: the updates the other side lacks.
    SyncUpdates {
        updates: Vec<u8>,
    },
    /// Incremental CRDT update.
    Update {
        bytes: Vec<u8>,
    },
    /// Server → client: the update was validated and persisted at `server_seq`.
    Ack {
        server_seq: u64,
    },
    /// Presence (§20.7), never persisted.
    Ephemeral {
        bytes: Vec<u8>,
    },
    RequestSnapshot,
    Error {
        code: ErrorCode,
        message: String,
    },
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("frame truncated")]
    Truncated,
    #[error("not a Flowster sync frame")]
    BadMagic,
    #[error("protocol version {0} is not supported")]
    UnsupportedVersion(u8),
    #[error("unknown frame tag {0}")]
    UnknownTag(u8),
    #[error("unknown role {0}")]
    UnknownRole(u8),
    #[error("frame larger than {MAX_FRAME_BYTES} bytes")]
    TooLarge,
    #[error("invalid UTF-8 in string field")]
    BadUtf8,
    #[error("varint overflow")]
    VarintOverflow,
    #[error("{0} trailing bytes after frame")]
    Trailing(usize),
}

mod tag {
    pub const HELLO: u8 = 1;
    pub const WELCOME: u8 = 2;
    pub const SYNC_UPDATES: u8 = 3;
    pub const UPDATE: u8 = 4;
    pub const ACK: u8 = 5;
    pub const EPHEMERAL: u8 = 6;
    pub const REQUEST_SNAPSHOT: u8 = 7;
    pub const ERROR: u8 = 8;
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16);
        out.extend_from_slice(&MAGIC);
        out.push(PROTOCOL_VERSION);
        match self {
            Frame::Hello { doc_id, token, client_version_vector } => {
                out.push(tag::HELLO);
                put_bytes(&mut out, doc_id.as_bytes());
                put_bytes(&mut out, token.as_bytes());
                put_bytes(&mut out, client_version_vector);
            }
            Frame::Welcome { role, server_version_vector } => {
                out.push(tag::WELCOME);
                out.push(match role {
                    Role::Owner => 0,
                    Role::Editor => 1,
                    Role::Commenter => 2,
                    Role::Viewer => 3,
                });
                put_bytes(&mut out, server_version_vector);
            }
            Frame::SyncUpdates { updates } => {
                out.push(tag::SYNC_UPDATES);
                put_bytes(&mut out, updates);
            }
            Frame::Update { bytes } => {
                out.push(tag::UPDATE);
                put_bytes(&mut out, bytes);
            }
            Frame::Ack { server_seq } => {
                out.push(tag::ACK);
                put_varint(&mut out, *server_seq);
            }
            Frame::Ephemeral { bytes } => {
                out.push(tag::EPHEMERAL);
                put_bytes(&mut out, bytes);
            }
            Frame::RequestSnapshot => out.push(tag::REQUEST_SNAPSHOT),
            Frame::Error { code, message } => {
                out.push(tag::ERROR);
                put_varint(&mut out, u64::from(code.to_u16()));
                put_bytes(&mut out, message.as_bytes());
            }
        }
        out
    }

    pub fn decode(input: &[u8]) -> Result<Frame, DecodeError> {
        if input.len() > MAX_FRAME_BYTES {
            return Err(DecodeError::TooLarge);
        }
        let mut r = Reader { buf: input, pos: 0 };
        if r.take(2)? != MAGIC {
            return Err(DecodeError::BadMagic);
        }
        let version = r.u8()?;
        if !(MIN_SUPPORTED_VERSION..=PROTOCOL_VERSION).contains(&version) {
            return Err(DecodeError::UnsupportedVersion(version));
        }
        let frame = match r.u8()? {
            tag::HELLO => Frame::Hello { doc_id: r.string()?, token: r.string()?, client_version_vector: r.bytes()? },
            tag::WELCOME => {
                let role = match r.u8()? {
                    0 => Role::Owner,
                    1 => Role::Editor,
                    2 => Role::Commenter,
                    3 => Role::Viewer,
                    x => return Err(DecodeError::UnknownRole(x)),
                };
                Frame::Welcome { role, server_version_vector: r.bytes()? }
            }
            tag::SYNC_UPDATES => Frame::SyncUpdates { updates: r.bytes()? },
            tag::UPDATE => Frame::Update { bytes: r.bytes()? },
            tag::ACK => Frame::Ack { server_seq: r.varint()? },
            tag::EPHEMERAL => Frame::Ephemeral { bytes: r.bytes()? },
            tag::REQUEST_SNAPSHOT => Frame::RequestSnapshot,
            tag::ERROR => {
                let code = u16::try_from(r.varint()?).map_err(|_| DecodeError::VarintOverflow)?;
                Frame::Error { code: ErrorCode::from_u16(code), message: r.string()? }
            }
            t => return Err(DecodeError::UnknownTag(t)),
        };
        match input.len() - r.pos {
            0 => Ok(frame),
            n => Err(DecodeError::Trailing(n)),
        }
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::Truncated)?;
        let s = self.buf.get(self.pos..end).ok_or(DecodeError::Truncated)?;
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn varint(&mut self) -> Result<u64, DecodeError> {
        let mut v: u64 = 0;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            let low = u64::from(b & 0x7f);
            if shift == 63 && low > 1 {
                return Err(DecodeError::VarintOverflow);
            }
            v |= low << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(DecodeError::VarintOverflow)
    }
    fn bytes(&mut self) -> Result<Vec<u8>, DecodeError> {
        let n = usize::try_from(self.varint()?).map_err(|_| DecodeError::TooLarge)?;
        Ok(self.take(n)?.to_vec())
    }
    fn string(&mut self) -> Result<String, DecodeError> {
        String::from_utf8(self.bytes()?).map_err(|_| DecodeError::BadUtf8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<Frame> {
        vec![
            Frame::Hello { doc_id: "doc-1".into(), token: "t".into(), client_version_vector: vec![1, 2, 3] },
            Frame::Welcome { role: Role::Commenter, server_version_vector: vec![] },
            Frame::SyncUpdates { updates: vec![9; 300] },
            Frame::Update { bytes: vec![0xff, 0] },
            Frame::Ack { server_seq: u64::MAX },
            Frame::Ephemeral { bytes: b"cursor".to_vec() },
            Frame::RequestSnapshot,
            Frame::Error { code: ErrorCode::ClientTooOld, message: "upgrade".into() },
            Frame::Error { code: ErrorCode::Unknown(999), message: String::new() },
        ]
    }

    #[test]
    fn round_trips() {
        for f in all() {
            assert_eq!(Frame::decode(&f.encode()).unwrap(), f);
        }
    }

    #[test]
    fn rejects_malformed_input_without_panicking() {
        for f in all() {
            let bytes = f.encode();
            for cut in 0..bytes.len() {
                assert!(Frame::decode(&bytes[..cut]).is_err(), "{f:?} cut at {cut}");
            }
            let mut extra = bytes.clone();
            extra.push(0);
            assert_eq!(Frame::decode(&extra), Err(DecodeError::Trailing(1)));
        }
        assert_eq!(Frame::decode(b"XX\x01\x07"), Err(DecodeError::BadMagic));
        assert_eq!(Frame::decode(b"FS\x63\x07"), Err(DecodeError::UnsupportedVersion(0x63)));
        assert_eq!(Frame::decode(b"FS\x01\xee"), Err(DecodeError::UnknownTag(0xee)));
        // Length prefix far beyond the buffer.
        assert_eq!(Frame::decode(b"FS\x01\x04\xff\xff\xff\xff\x0f"), Err(DecodeError::Truncated));
        assert_eq!(
            Frame::decode(b"FS\x01\x05\xff\xff\xff\xff\xff\xff\xff\xff\xff\x7f"),
            Err(DecodeError::VarintOverflow)
        );
    }
}
