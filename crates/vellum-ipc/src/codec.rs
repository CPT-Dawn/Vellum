//! Frame headers and the hand-written field encoding.

use crate::Error;

/// The first four bytes of every frame.
pub const MAGIC: [u8; 4] = *b"VLUM";
/// Bump on any incompatible change to a payload layout or message kind.
pub const PROTOCOL_VERSION: u16 = 3;
/// Length of the fixed frame header.
pub const HEADER_LEN: usize = 12;
/// Upper bound on a payload. Pixels never travel inline (they go as a memfd),
/// so real messages are tiny.
pub const MAX_PAYLOAD: u32 = 1 << 20;

/// A parsed frame header.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Header {
    pub kind: u16,
    pub len: u32,
}

impl Header {
    pub fn parse(bytes: &[u8; HEADER_LEN]) -> Result<Header, Error> {
        if bytes[0..4] != MAGIC {
            return Err(Error::BadMagic);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != PROTOCOL_VERSION {
            return Err(Error::VersionMismatch {
                ours: PROTOCOL_VERSION,
                theirs: version,
            });
        }
        let kind = u16::from_le_bytes([bytes[6], bytes[7]]);
        let len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        if len > MAX_PAYLOAD {
            return Err(Error::TooLarge(len));
        }
        Ok(Header { kind, len })
    }
}

/// A message that can be sent with [`send`](crate::send) and received with
/// [`recv`](crate::recv): [`Request`](crate::Request) or
/// [`Response`](crate::Response). Sealed.
pub trait Message: Body {}

impl Message for crate::Request {}
impl Message for crate::Response {}

/// The encoding half of [`Message`]. Lives in a private module, which seals
/// [`Message`].
pub trait Body: Sized {
    /// The kind written in the frame header.
    fn kind(&self) -> u16;
    /// Writes the payload fields.
    fn write(&self, w: &mut Writer);
    /// Reads the payload fields of a message of `kind`.
    fn read(kind: u16, r: &mut Reader<'_>) -> Result<Self, Error>;
}

pub(crate) fn encode<M: Body>(message: &M) -> Vec<u8> {
    let mut w = Writer(Vec::with_capacity(64));
    w.0.extend_from_slice(&MAGIC);
    w.u16(PROTOCOL_VERSION);
    w.u16(message.kind());
    w.u32(0); // patched below
    message.write(&mut w);
    let len = (w.0.len() - HEADER_LEN) as u32;
    w.0[8..12].copy_from_slice(&len.to_le_bytes());
    w.0
}

pub(crate) fn decode<M: Body>(frame: &[u8]) -> Result<M, Error> {
    let header: &[u8; HEADER_LEN] = frame
        .get(..HEADER_LEN)
        .and_then(|h| h.try_into().ok())
        .ok_or_else(|| {
            // A short frame that already fails the magic check is garbage,
            // not a truncated Vellum frame.
            if frame.len() >= 4 && frame[0..4] != MAGIC {
                Error::BadMagic
            } else {
                Error::Truncated
            }
        })?;
    let header = Header::parse(header)?;
    decode_payload(header, &frame[HEADER_LEN..])
}

pub(crate) fn decode_payload<M: Body>(header: Header, payload: &[u8]) -> Result<M, Error> {
    let payload = payload.get(..header.len as usize).ok_or(Error::Truncated)?;
    let mut r = Reader(payload);
    M::read(header.kind, &mut r)
}

/// Appends little-endian fields to a payload.
pub struct Writer(Vec<u8>);

impl Writer {
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    /// A `u32` byte length followed by UTF-8 bytes.
    pub fn str(&mut self, v: &str) {
        self.u32(v.len() as u32);
        self.0.extend_from_slice(v.as_bytes());
    }
}

/// Reads little-endian fields from a payload.
pub struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if self.0.len() < n {
            return Err(Error::Truncated);
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }
    pub fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, Error> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    pub fn u32(&mut self) -> Result<u32, Error> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn u64(&mut self) -> Result<u64, Error> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes(b.try_into().expect("took 8 bytes")))
    }
    pub fn string(&mut self) -> Result<String, Error> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| Error::BadUtf8)
    }
}
