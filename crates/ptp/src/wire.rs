//! The PTP wire format: USB containers, operation and response codes, PTP strings and arrays, and
//! the DeviceInfo and ObjectInfo datasets. All little-endian; every read is bounds-checked, so a
//! camera's garbage is an [`Error::Protocol`], never a panic.

use crate::Error;

/// Container types (the `type` field of a USB container).
pub const KIND_COMMAND: u16 = 1;
pub const KIND_DATA: u16 = 2;
pub const KIND_RESPONSE: u16 = 3;
pub const KIND_EVENT: u16 = 4;

/// The 12-byte container header.
pub const HEADER_LEN: usize = 12;

// operations
pub const OP_GET_DEVICE_INFO: u16 = 0x1001;
pub const OP_OPEN_SESSION: u16 = 0x1002;
pub const OP_CLOSE_SESSION: u16 = 0x1003;
pub const OP_GET_STORAGE_IDS: u16 = 0x1004;
pub const OP_GET_OBJECT_HANDLES: u16 = 0x1007;
pub const OP_GET_OBJECT_INFO: u16 = 0x1008;
pub const OP_GET_OBJECT: u16 = 0x1009;
pub const OP_GET_THUMB: u16 = 0x100A;

// responses
pub const RESP_OK: u16 = 0x2001;
pub const RESP_SESSION_NOT_OPEN: u16 = 0x2003;
pub const RESP_INVALID_PARAMETER: u16 = 0x201D;
pub const RESP_SESSION_ALREADY_OPEN: u16 = 0x201E;

/// Object format of a folder.
pub const FORMAT_ASSOCIATION: u16 = 0x3001;
/// "Every object" for GetObjectHandles' format and parent parameters.
pub const ALL: u32 = 0xFFFF_FFFF;

/// A human name for a response code.
pub fn response_name(code: u16) -> String {
    let name = match code {
        0x2000 => "undefined",
        RESP_OK => "OK",
        0x2002 => "general error",
        RESP_SESSION_NOT_OPEN => "session not open",
        0x2004 => "invalid transaction",
        0x2005 => "operation not supported",
        0x2006 => "parameter not supported",
        0x2007 => "incomplete transfer",
        0x2008 => "invalid storage",
        0x2009 => "invalid object handle",
        0x200A => "device property not supported",
        0x200B => "invalid object format",
        0x200C => "store full",
        0x200D => "object write-protected",
        0x200E => "store read-only",
        0x200F => "access denied",
        0x2010 => "no thumbnail",
        0x2012 => "partial deletion",
        0x2013 => "store not available",
        0x2019 => "device busy",
        0x201A => "invalid parent",
        RESP_INVALID_PARAMETER => "invalid parameter",
        RESP_SESSION_ALREADY_OPEN => "session already open",
        _ => "",
    };
    if name.is_empty() { format!("response 0x{code:04X}") } else { format!("{name}, 0x{code:04X}") }
}

/// A container header as read from the bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// The container's whole length, header included.
    pub len: u32,
    pub kind: u16,
    /// Operation, response or event code.
    pub code: u16,
    pub transaction: u32,
}

impl Header {
    /// The header at the start of `buf` (`None` when there are not 12 bytes).
    pub fn parse(buf: &[u8]) -> Option<Header> {
        let mut c = Cursor::new(buf);
        let len = c.u32().ok()?;
        let kind = c.u16().ok()?;
        let code = c.u16().ok()?;
        let transaction = c.u32().ok()?;
        Some(Header { len, kind, code, transaction })
    }

    /// The bytes after the header, by the header's length.
    pub fn payload_len(&self) -> usize {
        (self.len as usize).saturating_sub(HEADER_LEN)
    }

    pub fn to_bytes(self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0..4].copy_from_slice(&self.len.to_le_bytes());
        out[4..6].copy_from_slice(&self.kind.to_le_bytes());
        out[6..8].copy_from_slice(&self.code.to_le_bytes());
        out[8..12].copy_from_slice(&self.transaction.to_le_bytes());
        out
    }
}

/// A command container: `code` with up to five `u32` parameters.
pub fn command(code: u16, transaction: u32, params: &[u32]) -> Vec<u8> {
    let params = &params[..params.len().min(5)];
    let len = HEADER_LEN.saturating_add(params.len().saturating_mul(4));
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(&Header { len: len as u32, kind: KIND_COMMAND, code, transaction }.to_bytes());
    for p in params {
        out.extend_from_slice(&p.to_le_bytes());
    }
    out
}

/// A data container around `payload` (the header, then the bytes).
pub fn data(code: u16, transaction: u32, payload: &[u8]) -> Vec<u8> {
    let len = HEADER_LEN.saturating_add(payload.len());
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(&Header { len: len as u32, kind: KIND_DATA, code, transaction }.to_bytes());
    out.extend_from_slice(payload);
    out
}

/// The `u32` parameters of a response payload (whatever whole words it holds).
pub fn params(payload: &[u8]) -> Vec<u32> {
    payload.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

/// A bounds-checked little-endian reader.
pub struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Cursor { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).ok_or_else(|| short("length overflow"))?;
        let s = self.buf.get(self.pos..end).ok_or_else(|| short("dataset ends early"))?;
        self.pos = end;
        Ok(s)
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

    /// A PTP string: a character count (NUL included), then that many UTF-16LE code units.
    pub fn string(&mut self) -> Result<String, Error> {
        let n = usize::from(self.u8()?);
        if n == 0 {
            return Ok(String::new());
        }
        let b = self.take(n.saturating_mul(2))?;
        let units: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        Ok(String::from_utf16_lossy(&units[..end]))
    }

    /// A PTP array of `u16`: a `u32` count, then the elements. Capped by what the buffer holds.
    pub fn array16(&mut self) -> Result<Vec<u16>, Error> {
        let n = self.u32()? as usize;
        let n = n.min(self.buf.len().saturating_sub(self.pos) / 2);
        (0..n).map(|_| self.u16()).collect()
    }

    /// A PTP array of `u32`.
    pub fn array32(&mut self) -> Result<Vec<u32>, Error> {
        let n = self.u32()? as usize;
        let n = n.min(self.buf.len().saturating_sub(self.pos) / 4);
        (0..n).map(|_| self.u32()).collect()
    }
}

fn short(what: &str) -> Error {
    Error::Protocol(what.into())
}

/// The DeviceInfo dataset (what GetDeviceInfo returns).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceInfo {
    pub manufacturer: String,
    pub model: String,
    pub version: String,
    pub serial: String,
    pub operations: Vec<u16>,
}

impl DeviceInfo {
    pub fn parse(buf: &[u8]) -> Result<DeviceInfo, Error> {
        let mut c = Cursor::new(buf);
        let _standard_version = c.u16()?;
        let _vendor_extension_id = c.u32()?;
        let _vendor_extension_version = c.u16()?;
        let _vendor_extension_desc = c.string()?;
        let _functional_mode = c.u16()?;
        let operations = c.array16()?;
        let _events = c.array16()?;
        let _device_properties = c.array16()?;
        let _capture_formats = c.array16()?;
        let _image_formats = c.array16()?;
        let manufacturer = c.string()?;
        let model = c.string()?;
        let version = c.string()?;
        let serial = c.string()?;
        Ok(DeviceInfo { manufacturer, model, version, serial, operations })
    }
}

/// The ObjectInfo dataset (what GetObjectInfo returns), the fields the import needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObjectInfo {
    pub storage: u32,
    pub format: u16,
    pub size: u32,
    pub parent: u32,
    pub filename: String,
    pub captured: String,
}

impl ObjectInfo {
    pub fn parse(buf: &[u8]) -> Result<ObjectInfo, Error> {
        let mut c = Cursor::new(buf);
        let storage = c.u32()?;
        let format = c.u16()?;
        let _protection = c.u16()?;
        let size = c.u32()?;
        let _thumb_format = c.u16()?;
        let _thumb_size = c.u32()?;
        let _thumb_width = c.u32()?;
        let _thumb_height = c.u32()?;
        let _image_width = c.u32()?;
        let _image_height = c.u32()?;
        let _image_depth = c.u32()?;
        let parent = c.u32()?;
        let _association_type = c.u16()?;
        let _association_desc = c.u32()?;
        let _sequence = c.u32()?;
        let filename = c.string()?;
        let captured = c.string()?;
        Ok(ObjectInfo { storage, format, size, parent, filename, captured })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ptp_string(s: &str) -> Vec<u8> {
        if s.is_empty() {
            return vec![0];
        }
        let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        let mut out = vec![units.len() as u8];
        for u in units {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out
    }

    fn array16(v: &[u16]) -> Vec<u8> {
        let mut out = (v.len() as u32).to_le_bytes().to_vec();
        for x in v {
            out.extend_from_slice(&x.to_le_bytes());
        }
        out
    }

    #[test]
    fn a_command_container_has_the_header_and_its_parameters() {
        let c = command(OP_GET_OBJECT_HANDLES, 7, &[0x0001_0001, ALL, ALL]);
        assert_eq!(c.len(), 24);
        let h = Header::parse(&c).unwrap();
        assert_eq!(h, Header { len: 24, kind: KIND_COMMAND, code: OP_GET_OBJECT_HANDLES, transaction: 7 });
        assert_eq!(params(&c[HEADER_LEN..]), vec![0x0001_0001, ALL, ALL]);
        assert_eq!(command(OP_OPEN_SESSION, 0, &[1]).len(), 16);
    }

    #[test]
    fn a_header_needs_twelve_bytes() {
        assert_eq!(Header::parse(&[1, 2, 3]), None);
        let h = Header { len: 12, kind: KIND_RESPONSE, code: RESP_OK, transaction: 3 };
        assert_eq!(Header::parse(&h.to_bytes()), Some(h));
        assert_eq!(h.payload_len(), 0);
        assert_eq!(Header { len: 5, ..h }.payload_len(), 0, "a length below the header is no payload, not an underflow");
    }

    #[test]
    fn device_info_parses_strings_and_the_operation_list() {
        let mut b = Vec::new();
        b.extend_from_slice(&100u16.to_le_bytes()); // standard version
        b.extend_from_slice(&0u32.to_le_bytes()); // vendor extension id
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend(ptp_string(""));
        b.extend_from_slice(&0u16.to_le_bytes()); // functional mode
        b.extend(array16(&[OP_GET_DEVICE_INFO, OP_OPEN_SESSION, OP_GET_OBJECT]));
        b.extend(array16(&[]));
        b.extend(array16(&[]));
        b.extend(array16(&[]));
        b.extend(array16(&[0x3801]));
        b.extend(ptp_string("Canon Inc."));
        b.extend(ptp_string("Canon EOS RP"));
        b.extend(ptp_string("3-1.6.0"));
        b.extend(ptp_string("7a2d88da"));
        let info = DeviceInfo::parse(&b).unwrap();
        assert_eq!(info.manufacturer, "Canon Inc.");
        assert_eq!(info.model, "Canon EOS RP");
        assert_eq!(info.serial, "7a2d88da");
        assert_eq!(info.operations, vec![OP_GET_DEVICE_INFO, OP_OPEN_SESSION, OP_GET_OBJECT]);
        // cut short anywhere: an error, never a panic
        for n in 0..b.len() {
            assert!(DeviceInfo::parse(&b[..n]).is_err(), "{n} bytes");
        }
    }

    #[test]
    fn object_info_parses_a_photo_and_a_folder() {
        let mut b = Vec::new();
        b.extend_from_slice(&0x0001_0001u32.to_le_bytes()); // storage
        b.extend_from_slice(&0xB108u16.to_le_bytes()); // Canon CR3
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&28_312_441u32.to_le_bytes()); // size
        b.extend_from_slice(&0x3808u16.to_le_bytes()); // thumb format
        for _ in 0..6 {
            b.extend_from_slice(&0u32.to_le_bytes()); // thumb size/w/h, image w/h/depth
        }
        b.extend_from_slice(&0x42u32.to_le_bytes()); // parent
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend(ptp_string("IMG_0001.CR3"));
        b.extend(ptp_string("20260312T142501"));
        b.extend(ptp_string(""));
        let o = ObjectInfo::parse(&b).unwrap();
        assert_eq!((o.storage, o.format, o.size, o.parent), (0x0001_0001, 0xB108, 28_312_441, 0x42));
        assert_eq!(o.filename, "IMG_0001.CR3");
        assert_eq!(o.captured, "20260312T142501");
        assert!(ObjectInfo::parse(&b[..40]).is_err());
    }

    #[test]
    fn arrays_are_capped_by_the_bytes_there_are() {
        // a count of a billion with four bytes of elements
        let mut b = (1_000_000_000u32).to_le_bytes().to_vec();
        b.extend_from_slice(&[1, 0, 0, 0]);
        assert_eq!(Cursor::new(&b).array32().unwrap(), vec![1]);
        assert_eq!(Cursor::new(&b).array16().unwrap(), vec![1, 0]);
    }

    #[test]
    fn response_names_read_well() {
        assert_eq!(response_name(RESP_OK), "OK, 0x2001");
        assert_eq!(response_name(0x2019), "device busy, 0x2019");
        assert_eq!(response_name(0xA001), "response 0xA001");
    }
}
