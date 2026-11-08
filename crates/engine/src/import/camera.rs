//! Import candidates for the files on a camera over USB (PTP). Their paths are
//! `ptp://<camera id>/<object handle>/<DCIM/100CANON/IMG_0001.CR3>`: not files on disk, so the
//! review draws their thumbnails from the camera and the import copies them first.

use lightcraft_catalog::{Catalog, MediaKind};
use lightcraft_ptp::Object;

use super::ImportCandidate;

/// The scheme of a camera object's path.
pub const PTP_PREFIX: &str = "ptp://";

/// Raw file extensions (lower case): what imports as [`MediaKind::Raw`].
const RAW_EXTENSIONS: &[&str] = &[
    "dng", "cr2", "cr3", "nef", "nrw", "arw", "sr2", "srf", "raf", "orf", "rw2", "rwl", "raw", "pef", "srw", "iiq", "crw", "mrw", "x3f", "kdc",
    "mos", "erf", "3fr", "fff", "mef",
];

/// The path of object `handle` on `camera`, placed at `relative` (its folders and name).
pub fn ptp_path(camera: &str, handle: u32, relative: &str) -> String {
    format!("{PTP_PREFIX}{camera}/{handle}/{relative}")
}

/// `(camera id, handle, relative path)` of a [`ptp_path`]; `None` for any other path.
pub fn parse_ptp_path(path: &str) -> Option<(&str, u32, &str)> {
    let rest = path.strip_prefix(PTP_PREFIX)?;
    let (camera, rest) = rest.split_once('/')?;
    let (handle, relative) = rest.split_once('/')?;
    let handle = handle.parse().ok()?;
    (!camera.is_empty()).then_some((camera, handle, relative))
}

/// Whether `path` names an object on a camera.
pub fn is_ptp_path(path: &str) -> bool {
    parse_ptp_path(path).is_some()
}

fn extension(name: &str) -> String {
    std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// A camera's capture date (`20260312T142501`, PTP's DateTime) as the catalog writes it
/// (`2026-03-12T14:25:01`); `None` for anything else.
pub fn captured_iso(ptp: &str) -> Option<String> {
    let s = ptp.trim();
    let (date, time) = s.split_once('T')?;
    if date.len() != 8 || time.len() < 6 || !date.bytes().all(|b| b.is_ascii_digit()) || !time.as_bytes()[..6].iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(format!("{}-{}-{}T{}:{}:{}", &date[..4], &date[4..6], &date[6..8], &time[..2], &time[2..4], &time[4..6]))
}

/// The review's candidates for `objects` on `camera`: the supported files, oldest first. A file the
/// library already has by name and size is marked a duplicate (unchecked, "already in the library").
pub fn camera_candidates(camera: &str, objects: &[Object], library: &Catalog) -> Vec<ImportCandidate> {
    let mut out: Vec<ImportCandidate> = objects
        .iter()
        .filter(|o| !o.is_folder() && super::is_supported(std::path::Path::new(&o.name)))
        .map(|o| {
            let ext = extension(&o.name);
            let kind = if RAW_EXTENSIONS.contains(&ext.as_str()) { MediaKind::Raw } else { MediaKind::Image };
            let existing =
                library.photos().find(|p| !p.deleted && p.file_size == o.size && p.file_name.eq_ignore_ascii_case(&o.name)).map(|p| p.id.0);
            ImportCandidate {
                path: ptp_path(camera, o.handle, &o.relative_path()),
                name: o.name.clone(),
                format: ext.to_uppercase(),
                kind,
                width: 0,
                height: 0,
                file_size: o.size,
                captured: o.captured.as_deref().and_then(captured_iso),
                duplicate: existing.map(|_| "content".to_string()),
                existing,
                error: None,
                preview_only: None,
            }
        })
        .collect();
    out.sort_by(|a, b| a.captured.cmp(&b.captured).then_with(|| a.name.cmp(&b.name)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(handle: u32, name: &str, size: u64, captured: &str) -> Object {
        Object {
            handle,
            storage: 0x10001,
            parent: 2,
            format: 0x3801,
            size,
            name: name.into(),
            captured: Some(captured.into()),
            folders: vec!["DCIM".into(), "100CANON".into()],
        }
    }

    #[test]
    fn ptp_paths_round_trip() {
        let p = ptp_path("usb:04a9:32e2:@1", 42, "DCIM/100CANON/IMG_0001.CR3");
        assert_eq!(p, "ptp://usb:04a9:32e2:@1/42/DCIM/100CANON/IMG_0001.CR3");
        assert_eq!(parse_ptp_path(&p), Some(("usb:04a9:32e2:@1", 42, "DCIM/100CANON/IMG_0001.CR3")));
        assert!(!is_ptp_path("/Users/me/Pictures/a.jpg"));
        assert_eq!(parse_ptp_path("ptp://x/notanumber/a.jpg"), None);
        assert_eq!(parse_ptp_path("ptp:///1/a.jpg"), None);
    }

    #[test]
    fn capture_dates_become_catalog_dates() {
        assert_eq!(captured_iso("20260312T142501").as_deref(), Some("2026-03-12T14:25:01"));
        assert_eq!(captured_iso("20260312T142501.5"), Some("2026-03-12T14:25:01".into()), "fractions are dropped");
        assert_eq!(captured_iso(""), None);
        assert_eq!(captured_iso("2026-03-12"), None);
    }

    #[test]
    fn candidates_are_the_supported_files_oldest_first_with_library_duplicates_marked() {
        let mut library = Catalog::default();
        let id = library.alloc_photo_id();
        let photo = lightcraft_catalog::Photo::new(
            id,
            lightcraft_catalog::Source::File { path: "/old/IMG_0002.CR3".into() },
            "IMG_0002.CR3",
            "CR3",
            6000,
            4000,
            "2026-03-12T14:26:01",
        );
        let mut photo = photo;
        photo.file_size = 20;
        library.apply(lightcraft_catalog::Op::AddPhoto { photo: Box::new(photo) }).unwrap();
        let objects = vec![
            Object { format: 0x3001, ..object(1, "100CANON", 0, "") },
            object(3, "IMG_0002.CR3", 20, "20260312T142601"),
            object(4, "IMG_0001.CR3", 10, "20260312T142501"),
            object(5, "IMG_0001.JPG", 5, "20260312T142501"),
            object(6, "MVI_0003.MP4", 99, "20260312T143001"),
        ];
        let c = camera_candidates("cam", &objects, &library);
        assert_eq!(
            c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["IMG_0001.CR3", "IMG_0001.JPG", "IMG_0002.CR3"],
            "videos and folders left out"
        );
        assert_eq!((c[0].kind, c[0].format.as_str(), c[0].file_size), (MediaKind::Raw, "CR3", 10));
        assert_eq!(c[1].kind, MediaKind::Image);
        assert_eq!(c[0].captured.as_deref(), Some("2026-03-12T14:25:01"));
        assert_eq!(c[0].path, "ptp://cam/4/DCIM/100CANON/IMG_0001.CR3");
        assert_eq!((c[2].duplicate.as_deref(), c[2].existing), (Some("content"), Some(id.0)), "same name and size as a library photo");
        assert_eq!(c[0].duplicate, None);
    }
}
