//! PTP over USB with `nusb`: finding Still Image devices, one session per opened camera, and the
//! command / data / response transaction on its bulk endpoints.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nusb::MaybeFuture;
use nusb::descriptors::TransferType;
use nusb::transfer::{Bulk, Direction, In, Out, TransferError};

use crate::wire::{self, Header};
use crate::{CameraInfo, Error, Object};

/// USB class of the Still Image Capture interface every PTP camera has.
const CLASS_STILL_IMAGE: u8 = 6;
/// Canon's vendor id: its cameras are listed even when the interface is marked vendor-specific.
const VENDOR_CANON: u16 = 0x04a9;
/// Bytes read per bulk transfer while a data phase runs.
const CHUNK: usize = 256 * 1024;
/// A transfer that takes longer than this has lost the camera.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Folders are followed this deep at most when a camera will not list everything at once.
const MAX_DEPTH: usize = 16;

/// Whether `info` is a camera: it has a Still Image interface, or is a Canon with an imaging-class
/// or vendor-specific interface.
fn is_camera(info: &nusb::DeviceInfo) -> bool {
    let still_image = info.interfaces().any(|i| i.class() == CLASS_STILL_IMAGE);
    still_image || (info.vendor_id() == VENDOR_CANON && info.interfaces().any(|i| i.class() == 0xff || i.class() == 0))
}

fn camera_id(info: &nusb::DeviceInfo) -> String {
    match info.serial_number() {
        Some(serial) if !serial.trim().is_empty() => format!("usb:{:04x}:{:04x}:{}", info.vendor_id(), info.product_id(), serial.trim()),
        _ => format!("usb:{:04x}:{:04x}:@{}", info.vendor_id(), info.product_id(), info.device_address()),
    }
}

fn camera_name(info: &nusb::DeviceInfo) -> String {
    let product = info.product_string().unwrap_or("").trim();
    let maker = info.manufacturer_string().unwrap_or("").trim();
    let maker_word = maker.split_whitespace().next().unwrap_or("");
    match (product.is_empty(), maker.is_empty()) {
        (true, true) => format!("USB camera {:04x}:{:04x}", info.vendor_id(), info.product_id()),
        (true, false) => format!("{maker} camera"),
        (false, _) if maker_word.is_empty() || product.to_lowercase().starts_with(&maker_word.to_lowercase()) => product.to_string(),
        (false, _) => format!("{maker_word} {product}"),
    }
}

fn io(e: impl std::fmt::Display) -> Error {
    Error::Io(e.to_string())
}

fn open_error(e: nusb::Error) -> Error {
    match e.kind() {
        nusb::ErrorKind::Busy | nusb::ErrorKind::PermissionDenied => Error::Busy(e.to_string()),
        nusb::ErrorKind::Disconnected | nusb::ErrorKind::NotFound => Error::Gone,
        _ => io(e),
    }
}

/// The cameras plugged in right now.
pub fn cameras() -> Vec<CameraInfo> {
    let Ok(list) = nusb::list_devices().wait() else { return Vec::new() };
    let mut out: Vec<CameraInfo> = list
        .filter(is_camera)
        .map(|info| CameraInfo {
            id: camera_id(&info),
            name: camera_name(&info),
            vendor_id: info.vendor_id(),
            product_id: info.product_id(),
            serial: info.serial_number().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    out
}

/// An open PTP session with one camera. Dropping it closes the session.
pub struct Camera {
    _device: nusb::Device,
    _interface: nusb::Interface,
    ep_in: nusb::Endpoint<Bulk, In>,
    ep_out: nusb::Endpoint<Bulk, Out>,
    transaction: u32,
    info: wire::DeviceInfo,
}

impl Camera {
    /// Open `camera` and start a session. [`Error::Busy`] when another program holds it.
    pub fn open(camera: &CameraInfo) -> Result<Camera, Error> {
        let list = nusb::list_devices().wait().map_err(io)?;
        let info = list.filter(is_camera).find(|d| camera_id(d) == camera.id).ok_or(Error::Gone)?;
        let device = info.open().wait().map_err(open_error)?;
        let config = device.active_configuration().map_err(io)?;
        // the Still Image interface (or the first with a bulk pair) and its bulk endpoints
        let mut chosen = None;
        for iface in config.interfaces() {
            let alt = iface.first_alt_setting();
            let bulk_in = alt.endpoints().find(|e| e.direction() == Direction::In && e.transfer_type() == TransferType::Bulk).map(|e| e.address());
            let bulk_out = alt.endpoints().find(|e| e.direction() == Direction::Out && e.transfer_type() == TransferType::Bulk).map(|e| e.address());
            if let (Some(i), Some(o)) = (bulk_in, bulk_out) {
                let preferred = alt.class() == CLASS_STILL_IMAGE;
                if preferred || chosen.is_none() {
                    chosen = Some((iface.interface_number(), i, o));
                }
                if preferred {
                    break;
                }
            }
        }
        let (number, addr_in, addr_out) = chosen.ok_or_else(|| Error::Protocol("no bulk endpoints for pictures".into()))?;
        let interface = device.claim_interface(number).wait().map_err(open_error)?;
        let ep_in = interface.endpoint::<Bulk, In>(addr_in).map_err(io)?;
        let ep_out = interface.endpoint::<Bulk, Out>(addr_out).map_err(io)?;
        let mut cam = Camera { _device: device, _interface: interface, ep_in, ep_out, transaction: 0, info: wire::DeviceInfo::default() };
        // GetDeviceInfo is allowed outside a session; OpenSession(1), closing a stale one first
        let (data, _) = cam.transact(wire::OP_GET_DEVICE_INFO, &[], None, None)?;
        cam.info = wire::DeviceInfo::parse(&data)?;
        match cam.transact(wire::OP_OPEN_SESSION, &[1], None, None) {
            Ok(_) => {}
            Err(Error::Response(wire::RESP_SESSION_ALREADY_OPEN)) => {
                let _ = cam.transact(wire::OP_CLOSE_SESSION, &[], None, None);
                cam.transact(wire::OP_OPEN_SESSION, &[1], None, None)?;
            }
            Err(e) => return Err(e),
        }
        Ok(cam)
    }

    /// The camera's own description: maker, model, firmware, serial.
    pub fn device_info(&self) -> &wire::DeviceInfo {
        &self.info
    }

    /// Every file and folder on every storage, with each file's folders resolved.
    pub fn objects(&mut self) -> Result<Vec<Object>, Error> {
        let (data, _) = self.transact(wire::OP_GET_STORAGE_IDS, &[], None, None)?;
        let storages = wire::Cursor::new(&data).array32()?;
        let mut infos: HashMap<u32, (u32, wire::ObjectInfo)> = HashMap::new();
        for storage in storages {
            let handles = match self.handles(storage, 0) {
                Ok(h) => h,
                // a camera that will not list a storage at once: walk its folders
                Err(Error::Response(_)) => self.walk(storage)?,
                Err(e) => return Err(e),
            };
            for handle in handles {
                if infos.contains_key(&handle) {
                    continue;
                }
                let (data, _) = self.transact(wire::OP_GET_OBJECT_INFO, &[handle], None, None)?;
                infos.insert(handle, (storage, wire::ObjectInfo::parse(&data)?));
            }
        }
        let mut out: Vec<Object> = infos
            .iter()
            .map(|(&handle, (storage, o))| {
                let mut folders = Vec::new();
                let mut parent = o.parent;
                let mut seen = HashSet::new();
                while parent != 0 && seen.insert(parent) && folders.len() < MAX_DEPTH {
                    let Some((_, p)) = infos.get(&parent) else { break };
                    folders.push(p.filename.clone());
                    parent = p.parent;
                }
                folders.reverse();
                Object {
                    handle,
                    storage: *storage,
                    parent: o.parent,
                    format: o.format,
                    size: u64::from(o.size),
                    name: o.filename.clone(),
                    captured: Some(o.captured.clone()).filter(|c| !c.is_empty()),
                    folders,
                }
            })
            .collect();
        out.sort_by_key(Object::relative_path);
        Ok(out)
    }

    /// Copy object `handle` into `sink`. `cancel` stops it between chunks; `progress` gets the
    /// bytes written so far.
    pub fn download(&mut self, handle: u32, sink: &mut dyn Write, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<(), Error> {
        let mut written = 0u64;
        let mut on_chunk = |chunk: &[u8]| -> Result<(), Error> {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            sink.write_all(chunk).map_err(|e| Error::Io(format!("could not write the file: {e}")))?;
            written = written.saturating_add(chunk.len() as u64);
            progress(written);
            Ok(())
        };
        self.transact(wire::OP_GET_OBJECT, &[handle], Some(&mut on_chunk), Some(cancel))?;
        Ok(())
    }

    /// The camera's own thumbnail of object `handle` (a small JPEG, GetThumb).
    pub fn thumb(&mut self, handle: u32) -> Result<Vec<u8>, Error> {
        let (data, _) = self.transact(wire::OP_GET_THUMB, &[handle], None, None)?;
        if data.is_empty() {
            return Err(Error::Response(0x2010));
        }
        Ok(data)
    }

    /// The handles under `parent` on `storage` (0: every object on it).
    fn handles(&mut self, storage: u32, parent: u32) -> Result<Vec<u32>, Error> {
        let (data, _) = self.transact(wire::OP_GET_OBJECT_HANDLES, &[storage, 0, parent], None, None)?;
        wire::Cursor::new(&data).array32()
    }

    /// Every handle on `storage`, folder by folder from the root.
    fn walk(&mut self, storage: u32) -> Result<Vec<u32>, Error> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut pending = vec![(wire::ALL, 0usize)];
        while let Some((parent, depth)) = pending.pop() {
            if depth > MAX_DEPTH {
                continue;
            }
            for handle in self.handles(storage, parent)? {
                if !seen.insert(handle) {
                    continue;
                }
                out.push(handle);
                let (data, _) = self.transact(wire::OP_GET_OBJECT_INFO, &[handle], None, None)?;
                if wire::ObjectInfo::parse(&data)?.format == wire::FORMAT_ASSOCIATION {
                    pending.push((handle, depth.saturating_add(1)));
                }
            }
        }
        Ok(out)
    }

    fn next_transaction(&mut self) -> u32 {
        self.transaction = self.transaction.wrapping_add(1).max(1);
        self.transaction
    }

    /// One bulk read (up to [`CHUNK`] bytes).
    fn read_chunk(&mut self) -> Result<Vec<u8>, Error> {
        let buf = self.ep_in.allocate(CHUNK);
        let done = self.ep_in.transfer_blocking(buf, TIMEOUT);
        match done.status {
            Ok(()) => {}
            Err(TransferError::Disconnected) => return Err(Error::Gone),
            Err(TransferError::Cancelled) => return Err(Error::Io("the camera did not answer in time".into())),
            Err(TransferError::Stall) => {
                let _ = self.ep_in.clear_halt().wait();
                return Err(Error::Io("the camera stalled the transfer".into()));
            }
            Err(e) => return Err(io(e)),
        }
        let mut v = done.buffer.into_vec();
        v.truncate(done.actual_len);
        Ok(v)
    }

    fn write_all(&mut self, bytes: Vec<u8>) -> Result<(), Error> {
        let done = self.ep_out.transfer_blocking(bytes.into(), TIMEOUT);
        match done.status {
            Ok(()) => Ok(()),
            Err(TransferError::Disconnected) => Err(Error::Gone),
            Err(TransferError::Cancelled) => Err(Error::Io("the camera did not take the command in time".into())),
            Err(TransferError::Stall) => {
                let _ = self.ep_out.clear_halt().wait();
                Err(Error::Io("the camera stalled the command".into()))
            }
            Err(e) => Err(io(e)),
        }
    }

    /// Send `code` with `params`; collect the data phase (into `sink` when given, else returned)
    /// and the response parameters. A response other than OK is [`Error::Response`].
    fn transact(
        &mut self,
        code: u16,
        params: &[u32],
        mut sink: Option<&mut dyn FnMut(&[u8]) -> Result<(), Error>>,
        cancel: Option<&AtomicBool>,
    ) -> Result<(Vec<u8>, Vec<u32>), Error> {
        let tx = self.next_transaction();
        self.write_all(wire::command(code, tx, params))?;
        let mut data = Vec::new();
        let mut empty_reads = 0u32;
        loop {
            let chunk = self.read_chunk()?;
            if chunk.is_empty() {
                empty_reads = empty_reads.saturating_add(1);
                if empty_reads > 3 {
                    return Err(Error::Protocol("the camera sent nothing".into()));
                }
                continue;
            }
            let header = Header::parse(&chunk).ok_or_else(|| Error::Protocol("a container shorter than its header".into()))?;
            match header.kind {
                wire::KIND_DATA => {
                    let total = header.payload_len();
                    let mut got = 0usize;
                    let first = chunk.get(wire::HEADER_LEN..).unwrap_or(&[]);
                    let first = &first[..first.len().min(total)];
                    deliver(first, &mut sink, &mut data)?;
                    got = got.saturating_add(first.len());
                    let mut stalls = 0u32;
                    while got < total {
                        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                            return Err(Error::Cancelled);
                        }
                        let more = self.read_chunk()?;
                        if more.is_empty() {
                            stalls = stalls.saturating_add(1);
                            if stalls > 3 {
                                return Err(Error::Protocol("the data phase stopped early".into()));
                            }
                            continue;
                        }
                        let take = more.len().min(total.saturating_sub(got));
                        deliver(&more[..take], &mut sink, &mut data)?;
                        got = got.saturating_add(take);
                    }
                }
                wire::KIND_RESPONSE => {
                    if header.transaction != tx {
                        return Err(Error::Protocol(format!("response for transaction {} while waiting for {tx}", header.transaction)));
                    }
                    if header.code != wire::RESP_OK {
                        return Err(Error::Response(header.code));
                    }
                    let payload = chunk.get(wire::HEADER_LEN..).unwrap_or(&[]);
                    return Ok((data, wire::params(payload)));
                }
                // events travel on the interrupt endpoint; one on bulk is skipped
                wire::KIND_EVENT => {}
                other => return Err(Error::Protocol(format!("container type {other}"))),
            }
        }
    }
}

fn deliver(bytes: &[u8], sink: &mut Option<&mut dyn FnMut(&[u8]) -> Result<(), Error>>, data: &mut Vec<u8>) -> Result<(), Error> {
    match sink {
        Some(f) => f(bytes),
        None => {
            data.extend_from_slice(bytes);
            Ok(())
        }
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        let _ = self.transact(wire::OP_CLOSE_SESSION, &[], None, None);
    }
}
