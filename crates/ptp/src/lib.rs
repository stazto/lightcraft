//! Cameras over USB, in pure Rust: the Picture Transfer Protocol (PTP, ISO 15740, the Still Image
//! class every camera speaks; MTP is its superset) over the `nusb` USB library.
//!
//! [`cameras`] lists the cameras plugged in. [`Camera::open`] opens one and starts a session;
//! [`Camera::objects`] lists its files with their folders (`DCIM/100CANON/IMG_0001.CR3`), and
//! [`Camera::download`] copies one into a writer. The web build has no USB and lists nothing.
//!
//! The wire format lives in [`wire`] (containers, strings, dates, object info) and is unit-tested
//! on synthetic bytes; the USB transport is the thin [`usb`] layer over it.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::fmt;

pub mod wire;

#[cfg(not(target_arch = "wasm32"))]
mod usb;

#[cfg(not(target_arch = "wasm32"))]
pub use usb::{Camera, cameras};

/// A camera on the bus, before it is opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraInfo {
    /// A stable id for this camera on this host (vendor, product and serial number).
    pub id: String,
    /// "Canon EOS RP" (the product string, with the maker in front when it is not already).
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub serial: Option<String>,
}

/// One file or folder on the camera.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
    pub handle: u32,
    pub storage: u32,
    /// The handle of the folder it is in (0 at a storage's root).
    pub parent: u32,
    /// PTP object format code (0x3001 is a folder).
    pub format: u16,
    pub size: u64,
    pub name: String,
    /// The capture date as the camera wrote it (`20260312T142501`), when it gave one.
    pub captured: Option<String>,
    /// The folders above it on the camera, outermost first (`DCIM`, `100CANON`).
    pub folders: Vec<String>,
}

impl Object {
    pub fn is_folder(&self) -> bool {
        self.format == wire::FORMAT_ASSOCIATION
    }

    /// `DCIM/100CANON/IMG_0001.CR3`.
    pub fn relative_path(&self) -> String {
        let mut p = self.folders.join("/");
        if !p.is_empty() {
            p.push('/');
        }
        p.push_str(&self.name);
        p
    }
}

/// What went wrong talking to a camera. Every variant is something the user can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The camera is not plugged in any more.
    Gone,
    /// Another program holds the camera (on macOS, Image Capture or Photos take it first).
    Busy(String),
    /// The camera answered a command with a PTP failure code.
    Response(u16),
    /// The camera's answer did not follow the protocol.
    Protocol(String),
    /// A USB or system error.
    Io(String),
    /// Stopped on request.
    Cancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Gone => write!(f, "the camera was unplugged"),
            Error::Busy(what) => write!(f, "another program holds the camera ({what}); on macOS quit Image Capture and Photos, then try again"),
            Error::Response(code) => write!(f, "the camera refused ({})", wire::response_name(*code)),
            Error::Protocol(what) => write!(f, "the camera answered something unexpected: {what}"),
            Error::Io(what) => write!(f, "USB error: {what}"),
            Error::Cancelled => write!(f, "stopped"),
        }
    }
}

impl std::error::Error for Error {}

/// `cameras` on a build without USB.
#[cfg(target_arch = "wasm32")]
pub fn cameras() -> Vec<CameraInfo> {
    Vec::new()
}
