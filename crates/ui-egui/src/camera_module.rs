//! The Import module for a camera over USB (PTP), as Lightroom's import screen: the camera's
//! photos fill the window (the "+" button's menu, or `import.camera {camera}`), by capture date,
//! as a grid of thumbnails, a list or one photo at a time, each with a checkbox; the right-hand
//! panel holds the destination, folder and naming options, the album, the Import buttons and the
//! progress of the copy and the import. Mounted cards and folders keep the import review dialog.
//!
//! The copy and the import themselves are the review's ([`crate::camera_import`]): the picks are
//! copied from the camera into a staging folder (a row in the activity stack, cancellable), then
//! imported with mode copy; the module only watches them and shows a summary at the end. The
//! full-size photo of the loupe is fetched the same way, one file into the camera's `preview/`
//! folder, and rendered like any import candidate.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;

use lightcraft_engine::camera_fetch::State;
use lightcraft_engine::import::ImportCandidate;
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::dialogs::import::ImportDialog;
use crate::state::RightPanel;

mod center;
mod options;

pub use center::show_center;
pub use options::show_options;

/// The thumbnail slot the loupe's full photo is rendered into (the grid uses the candidates' indices).
pub const LOUPE_SLOT: u32 = u32::MAX - 1;

/// Thumbnail sizes the grid's slider runs over (points).
pub const THUMB_MIN: f32 = 90.0;
pub const THUMB_MAX: f32 = 320.0;
const THUMB_DEFAULT: f32 = 160.0;

/// How the photos are laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    /// Thumbnails by date, the size adjustable.
    #[default]
    Grid,
    /// One row per photo: name, date, size, format.
    List,
    /// One photo at a time, with previous / next.
    Loupe,
}

impl Layout {
    pub const ALL: [Layout; 3] = [Layout::Grid, Layout::List, Layout::Loupe];

    pub fn id(self) -> &'static str {
        match self {
            Layout::Grid => "grid",
            Layout::List => "list",
            Layout::Loupe => "loupe",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Layout::Grid => "Grid",
            Layout::List => "List",
            Layout::Loupe => "Photo",
        }
    }

    pub fn from_id(id: &str) -> Option<Layout> {
        Layout::ALL.into_iter().find(|l| l.id() == id)
    }
}

/// The camera being listed on a worker thread (reading a full card can take a while).
struct Listing {
    rx: Receiver<Result<Vec<ImportCandidate>, String>>,
    /// Its row in the activity stack.
    _guard: lightcraft_engine::activity::TaskGuard,
}

/// The loupe's full photo: fetched from the camera into the preview folder, then rendered.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    /// Which candidate.
    pub index: usize,
    /// Where its file lands.
    pub path: PathBuf,
    /// The candidate as a local file, once the copy is done (what the renderer draws).
    pub local: Option<ImportCandidate>,
    /// Why the copy did not end well.
    pub failed: Option<String>,
    /// A copy over USB is under way for it.
    pub fetching: bool,
}

/// An import started from the module: what it was asked and how far it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Run {
    /// How many photos were picked.
    pub planned: usize,
    /// The copy from the camera ended: how many files were written (and found there already).
    pub copied: Option<(usize, usize)>,
    /// The import of the copies ended: what it did.
    pub imported: usize,
    pub restored: usize,
    pub duplicates: usize,
    pub failed: usize,
    /// Everything has ended (well or not).
    pub done: bool,
    /// Stopped from the activity stack.
    pub cancelled: bool,
    /// Why it did not finish (a camera that went away, a refused copy).
    pub error: Option<String>,
}

impl Run {
    /// What the run did, in a sentence, once it is done.
    pub fn summary(&self) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        if self.cancelled && self.imported == 0 && self.restored == 0 {
            return crate::i18n::tr("The copy was stopped; nothing was imported").to_string();
        }
        let mut parts = vec![crate::i18n::tr_format!("{n} photo{} imported", crate::i18n::plural_s(self.imported), n = self.imported)];
        if self.restored > 0 {
            parts.push(crate::i18n::tr_format!("{n} restored", n = self.restored));
        }
        if self.duplicates > 0 {
            parts.push(crate::i18n::tr_format!("{n} already in the library", n = self.duplicates));
        }
        if self.failed > 0 {
            parts.push(crate::i18n::tr_format!("{n} failed", n = self.failed));
        }
        if self.cancelled {
            parts.push(crate::i18n::tr("stopped early").to_string());
        }
        parts.join(" · ")
    }
}

/// The module's state while it is open.
pub struct CameraModule {
    /// The camera's id (`Device::camera`).
    pub camera: String,
    /// Its name (the model, where the camera told it).
    pub name: String,
    /// The camera's photos, oldest first ([`lightcraft_engine::import::camera_candidates`]).
    pub candidates: Vec<ImportCandidate>,
    pub checked: Vec<bool>,
    /// The candidate clicked last: where a Shift-click range starts.
    pub last_clicked: Option<usize>,
    pub layout: Layout,
    /// The grid's thumbnail size (points).
    pub thumb_size: f32,
    /// The photo the loupe shows (an index into `candidates`).
    pub current: usize,
    /// Copy to this folder ("" = the library's Originals).
    pub destination: String,
    /// A subfolder inside the destination ("" = none); created by the import.
    pub subfolder: String,
    /// Below that, one folder per year and day.
    pub by_date: bool,
    /// The batch's name: the copies are renamed `<name>-0001`… ("" = keep the camera's names).
    pub batch: String,
    /// An existing album to add the photos to.
    pub album: Option<u64>,
    /// The camera is being listed.
    listing: Option<Listing>,
    /// The camera could not be read (Try Again lists it anew).
    pub error: Option<String>,
    /// The loupe's full photo.
    pub preview: Option<Preview>,
    /// Which candidate the loupe's texture holds.
    loupe_of: Option<usize>,
    /// The import started from here.
    pub run: Option<Run>,
    /// The right panel was closed when the module opened: it closes again with the module.
    restore_right: bool,
    /// Close at the end of the frame (a button inside the module asked).
    pub close_requested: bool,
    /// The last message a refused action left (shown in the options panel).
    pub notice: Option<String>,
}

impl CameraModule {
    fn new(camera: String, name: String) -> Self {
        CameraModule {
            camera,
            name,
            candidates: Vec::new(),
            checked: Vec::new(),
            last_clicked: None,
            layout: Layout::Grid,
            thumb_size: THUMB_DEFAULT,
            current: 0,
            destination: String::new(),
            subfolder: String::new(),
            by_date: true,
            batch: String::new(),
            album: None,
            listing: None,
            error: None,
            preview: None,
            loupe_of: None,
            run: None,
            restore_right: false,
            close_requested: false,
            notice: None,
        }
    }

    /// Candidate `i` can be imported: a file the library does not have.
    pub fn importable(&self, i: usize) -> bool {
        self.candidates.get(i).is_some_and(|c| c.error.is_none() && c.duplicate.is_none())
    }

    /// Candidate `i` is checked.
    pub fn is_checked(&self, i: usize) -> bool {
        self.checked.get(i).copied().unwrap_or(false) && self.importable(i)
    }

    /// How many are checked.
    pub fn checked_count(&self) -> usize {
        (0..self.candidates.len()).filter(|i| self.is_checked(*i)).count()
    }

    /// How many can be imported at all.
    pub fn importable_count(&self) -> usize {
        (0..self.candidates.len()).filter(|i| self.importable(*i)).count()
    }

    /// A click on candidate `i` toggles its checkbox; with Shift, every importable candidate from the
    /// last clicked one to `i` takes `i`'s new state.
    pub fn click(&mut self, i: usize, shift: bool) {
        if !self.importable(i) {
            return;
        }
        let Some(on) = self.checked.get(i).map(|c| !c) else { return };
        let from = if shift { self.last_clicked.filter(|a| *a < self.checked.len()).unwrap_or(i) } else { i };
        for k in from.min(i)..=from.max(i) {
            if self.importable(k)
                && let Some(c) = self.checked.get_mut(k)
            {
                *c = on;
            }
        }
        self.last_clicked = Some(i);
    }

    /// Check or uncheck every importable candidate.
    pub fn check_all(&mut self, on: bool) {
        for i in 0..self.candidates.len() {
            let importable = self.importable(i);
            if let Some(c) = self.checked.get_mut(i) {
                *c = on && importable;
            }
        }
    }

    /// The candidates by capture day, in order: `(day key, first index, count)`; a photo without a
    /// date is in the last group, keyed "".
    pub fn groups(&self) -> Vec<(String, usize, usize)> {
        let mut out: Vec<(String, usize, usize)> = Vec::new();
        for (i, c) in self.candidates.iter().enumerate() {
            let key = c.captured.as_deref().and_then(|d| d.get(..10)).unwrap_or("").to_string();
            match out.last_mut() {
                Some((k, _, n)) if *k == key => *n += 1,
                _ => out.push((key, i, 1)),
            }
        }
        out
    }

    /// The `library.import` folder choice: by day under the destination, flat, or inside the subfolder.
    pub fn organize(&self) -> (String, String) {
        let sub = self.subfolder.trim().trim_matches(['/', '\\']);
        match (sub.is_empty(), self.by_date) {
            (true, true) => ("date".into(), String::new()),
            (true, false) => ("flat".into(), String::new()),
            (false, true) => ("custom".into(), format!("{sub}/{{date:%Y}}/{{date:%Y-%m-%d}}")),
            (false, false) => ("custom".into(), sub.to_string()),
        }
    }

    /// The file-name template the batch name makes (`Lote` → `Lote-{seq:4}`); "" keeps the names.
    pub fn rename_template(&self) -> String {
        let b = self.batch.trim();
        if b.is_empty() {
            String::new()
        } else if b.contains('{') {
            b.to_string()
        } else {
            format!("{b}-{{seq:4}}")
        }
    }

    /// The import review settings the module stands for (what [`crate::import::start`] takes).
    fn review(&self, app: &LightcraftApp, all: bool) -> ImportDialog {
        let checked = (0..self.candidates.len()).map(|i| if all { self.importable(i) } else { self.is_checked(i) }).collect();
        let (organize, folder_template) = self.organize();
        ImportDialog {
            sources: vec![self.name.clone()],
            candidates: self.candidates.clone(),
            checked,
            copy: true,
            move_files: false,
            album: self.album,
            destination: self.destination.trim().to_string(),
            organize,
            folder_template,
            rename: self.rename_template(),
            on_deleted: "restore".into(),
            camera: Some(self.camera.clone()),
            library: Some(app.session.library_generation()),
            ..Default::default()
        }
    }

    /// Something is running for this camera: the listing, a copy or the import of the copies.
    pub fn busy(&self, app: &LightcraftApp) -> bool {
        self.listing.is_some() || app.camera_imports.iter().any(|p| p.camera == self.camera)
    }

    /// `ui.inspect` → `cameraModule`.
    pub fn status(&self, app: &LightcraftApp) -> Value {
        json!({
            "camera": self.camera,
            "name": self.name,
            "layout": self.layout.id(),
            "candidates": self.candidates.len(),
            "checked": self.checked_count(),
            "current": self.current,
            "listing": self.listing.is_some(),
            "error": self.error,
            "destination": self.destination,
            "subfolder": self.subfolder,
            "byDate": self.by_date,
            "batch": self.batch,
            "album": self.album,
            "busy": self.busy(app),
            "run": self.run.as_ref().map(|r| json!({"planned": r.planned, "copied": r.copied, "imported": r.imported, "duplicates": r.duplicates, "failed": r.failed, "done": r.done, "cancelled": r.cancelled, "error": r.error})),
            "preview": self.preview.as_ref().map(|p| json!({"index": p.index, "ready": p.local.is_some(), "fetching": p.fetching, "failed": p.failed})),
        })
    }
}

/// Whether the module is open.
pub fn is_open(app: &LightcraftApp) -> bool {
    app.camera_module.is_some()
}

/// Open the module on `device`, a camera over USB, listing it in the background; `album` is chosen
/// in the options. A module already open on another camera is replaced.
pub fn open(app: &mut LightcraftApp, device: &lightcraft_engine::devices::Device, album: Option<u64>) -> Result<Value, String> {
    let id = device.camera.clone().ok_or_else(|| format!("{} is not a camera over USB", device.name))?;
    if app.camera_module.as_ref().is_some_and(|m| m.camera == id) {
        if let Some(m) = app.camera_module.as_mut() {
            m.album = album.or(m.album);
        }
        return Ok(json!({"camera": id, "open": true}));
    }
    if app.camera_module.is_some() {
        close(app);
    }
    let mut m = CameraModule::new(id.clone(), device.name.clone());
    m.album = album;
    show(app, m);
    list(app);
    Ok(json!({"camera": id, "open": true, "listing": true}))
}

/// Open the module on `candidates` already listed (tests, and a listing done elsewhere).
pub fn open_with(app: &mut LightcraftApp, camera: &str, name: &str, candidates: Vec<ImportCandidate>) {
    if app.camera_module.is_some() {
        close(app);
    }
    let mut m = CameraModule::new(camera.to_string(), name.to_string());
    set_candidates(&mut m, candidates);
    show(app, m);
}

/// Put the module on screen: the right panel shows its options, so it is made visible.
fn show(app: &mut LightcraftApp, mut m: CameraModule) {
    m.thumb_size = app.ui.import_thumb_size.clamp(THUMB_MIN, THUMB_MAX);
    if app.ui.right == RightPanel::None {
        app.ui.right = RightPanel::Edit;
        m.restore_right = true;
    }
    app.renderer.forget_imports();
    app.camera_module = Some(m);
}

fn set_candidates(m: &mut CameraModule, candidates: Vec<ImportCandidate>) {
    m.checked = candidates.iter().map(|c| c.duplicate.is_none() && c.error.is_none()).collect();
    m.candidates = candidates;
    m.current = m.current.min(m.candidates.len().saturating_sub(1));
    m.last_clicked = None;
    m.preview = None;
    m.loupe_of = None;
}

/// Close the module: a copy under way goes on in the activity stack (the import review's rules
/// finish it); the loupe's fetch is stopped and the preview folder removed.
pub fn close(app: &mut LightcraftApp) {
    let Some(m) = app.camera_module.take() else { return };
    if m.preview.as_ref().is_some_and(|p| p.fetching) {
        app.session.cameras.cancel(&m.camera);
    }
    let _ = std::fs::remove_dir_all(preview_dir(&m.camera));
    app.ui.import_thumb_size = m.thumb_size;
    if m.restore_right {
        app.ui.right = RightPanel::None;
    }
    app.renderer.forget_imports();
}

/// `import.camera {camera, album?}`: the module on that camera (an id from `library.devices`).
/// `{demo: true}` opens it on the library's own files as if they were on a camera, and `{demo:
/// "<folder>"}` on that folder's photos with stand-in capture days (screenshots, scripts): nothing
/// is copied from anywhere, Import is refused.
pub fn command(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    let demo = p.get("demo").filter(|d| d.as_bool() == Some(true) || d.is_string());
    if let Some(demo) = demo {
        let candidates = match demo.as_str() {
            Some(folder) => demo_folder_candidates(folder)?,
            None => demo_candidates(app),
        };
        if candidates.is_empty() {
            return Err("import.camera: no files to stand for a camera".into());
        }
        let n = candidates.len();
        open_with(app, DEMO_CAMERA, "Demo Camera", candidates);
        return Ok(json!({"camera": DEMO_CAMERA, "open": true, "candidates": n}));
    }
    let id = p.get("camera").and_then(Value::as_str).ok_or("import.camera: `camera` (an id from library.devices) is needed")?;
    let device = lightcraft_engine::devices::devices()
        .into_iter()
        .chain(lightcraft_engine::devices::devices_now())
        .find(|d| d.camera.as_deref() == Some(id))
        .ok_or_else(|| format!("no camera {id} is plugged in"))?;
    open(app, &device, p.get("album").and_then(Value::as_u64))
}

/// The id of the stand-in camera `import.camera {demo: true}` opens.
pub const DEMO_CAMERA: &str = "demo";

/// The library's files (up to 60) as import candidates, for the demo module.
fn demo_candidates(app: &LightcraftApp) -> Vec<ImportCandidate> {
    let mut out: Vec<ImportCandidate> = app
        .session
        .catalog
        .photos()
        .filter(|p| !p.deleted)
        .filter_map(|p| match &p.source {
            lightcraft_catalog::Source::File { path } => Some(ImportCandidate {
                path: path.clone(),
                name: p.file_name.clone(),
                format: p.format.clone(),
                kind: p.kind,
                width: p.width,
                height: p.height,
                file_size: p.file_size,
                captured: p.captured.clone().filter(|c| !c.is_empty()),
                ..Default::default()
            }),
            _ => None,
        })
        .take(60)
        .collect();
    out.sort_by(|a, b| a.captured.cmp(&b.captured).then_with(|| a.name.cmp(&b.name)));
    out
}

/// The supported files of `folder` (not its subfolders) as import candidates, with stand-in
/// capture days (six photos a day from 6 November 2026), so the demo shows the day headers.
fn demo_folder_candidates(folder: &str) -> Result<Vec<ImportCandidate>, String> {
    let dir = std::fs::read_dir(folder).map_err(|e| format!("import.camera: {folder}: {e}"))?;
    let mut files: Vec<(String, u64)> = dir
        .flatten()
        .filter(|e| e.path().is_file() && lightcraft_engine::import::is_supported(&e.path()))
        .map(|e| (e.path().to_string_lossy().to_string(), e.metadata().map(|m| m.len()).unwrap_or(0)))
        .collect();
    files.sort();
    files.truncate(200);
    Ok(files
        .into_iter()
        .enumerate()
        .map(|(i, (path, size))| {
            let p = std::path::Path::new(&path);
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let ext = p.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
            let day = 6 + (i / 6).min(24);
            let hour = 9 + (i % 6) * 2;
            ImportCandidate {
                path,
                name,
                format: if ext == "JPEG" { "JPG".into() } else { ext },
                kind: lightcraft_catalog::MediaKind::Image,
                file_size: size,
                captured: Some(format!("2026-11-{day:02}T{hour:02}:00:00")),
                ..Default::default()
            }
        })
        .collect())
}

/// List the camera on a worker thread (again, after an error).
pub fn list(app: &mut LightcraftApp) {
    let Some(m) = app.camera_module.as_mut() else { return };
    if m.listing.is_some() || m.camera == DEMO_CAMERA {
        return;
    }
    m.error = None;
    let (tx, rx) = std::sync::mpsc::channel();
    let cameras = app.session.cameras.clone();
    let library = std::sync::Arc::new(app.session.catalog.clone());
    let id = m.camera.clone();
    let job = move || {
        let r = cameras.list(&id).map(|objects| lightcraft_engine::import::camera_candidates(&id, &objects, &library));
        let _ = tx.send(r);
    };
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(job);
    #[cfg(target_arch = "wasm32")]
    job();
    let guard = app.session.activity.start("scan", "Reading camera", lightcraft_engine::activity::Cancel::No);
    guard.detail(&m.name);
    m.listing = Some(Listing { rx, _guard: guard });
}

/// Where the loupe's full photos land: `<camera folder>/preview/`.
pub fn preview_dir(camera: &str) -> PathBuf {
    lightcraft_engine::devices::camera_root(camera).join("preview")
}

/// Start importing the checked photos (`all`: every importable one): the copy from the camera, then
/// the import of the copies, as the review does it.
pub fn start_import(app: &mut LightcraftApp, all: bool) -> Result<Value, String> {
    let Some(m) = app.camera_module.as_ref() else { return Err("the Import module is not open".into()) };
    if m.camera == DEMO_CAMERA {
        return Err(crate::i18n::tr("The demo camera's photos are the library's own: nothing to import").to_string());
    }
    if m.listing.is_some() {
        return Err(crate::i18n::tr("The camera is still being read").to_string());
    }
    if m.preview.as_ref().is_some_and(|p| p.fetching) {
        return Err(crate::i18n::tr("Wait for the photo to finish loading").to_string());
    }
    if m.run.as_ref().is_some_and(|r| !r.done) {
        return Err(crate::i18n::tr("An import from this camera is running").to_string());
    }
    let d = m.review(app, all);
    let planned = d.checked.iter().filter(|c| **c).count();
    if planned == 0 {
        return Err(crate::i18n::tr("No photos selected").to_string());
    }
    let r = crate::import::start(app, &d)?;
    if let Some(m) = app.camera_module.as_mut() {
        m.run = Some(Run { planned, ..Default::default() });
        m.notice = None;
    }
    Ok(r)
}

/// Make `i` the loupe's photo and fetch its file from the camera (unless it is here already).
pub fn show_photo(app: &mut LightcraftApp, i: usize) {
    let Some(m) = app.camera_module.as_mut() else { return };
    let Some(c) = m.candidates.get(i).cloned() else { return };
    m.current = i;
    if m.preview.as_ref().is_some_and(|p| p.index == i) {
        return;
    }
    // another photo's fetch under way: it stops (the next frame sees it end and goes on)
    if m.preview.as_ref().is_some_and(|p| p.fetching) {
        app.session.cameras.cancel(&m.camera);
        if let Some(p) = m.preview.as_mut() {
            p.index = i;
            p.failed = None;
            p.local = None;
        }
        return;
    }
    let Some((_, handle, _)) = lightcraft_engine::import::parse_ptp_path(&c.path) else {
        // a local file (tests): shown as it is
        m.preview = Some(Preview { index: i, path: PathBuf::from(&c.path), local: Some(c), failed: None, fetching: false });
        return;
    };
    let path = preview_dir(&m.camera).join(format!("{handle}-{}", safe_name(&c.name)));
    let here = std::fs::metadata(&path).is_ok_and(|md| md.is_file() && md.len() == c.file_size);
    let local = here.then(|| local_candidate(&c, &path));
    let mut preview = Preview { index: i, path: path.clone(), local, failed: None, fetching: false };
    if !here {
        // the camera is busy copying an import: the full photo comes once that is done
        if app.session.cameras.state_of(&m.camera).is_some() {
            preview.failed = Some(crate::i18n::tr("The camera is busy copying; the full photo follows when it is done").to_string());
        } else {
            let items = json!([{"handle": handle, "size": c.file_size, "name": c.name, "target": path.to_string_lossy()}]);
            match app.session.execute("library.cameraCopy", &json!({"camera": m.camera, "name": m.name, "items": items})) {
                Ok(_) => preview.fetching = true,
                Err(e) => preview.failed = Some(e.to_string()),
            }
        }
    }
    if let Some(m) = app.camera_module.as_mut() {
        m.preview = Some(preview);
    }
}

/// The camera's file name made safe for this file system.
fn safe_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') { '_' } else { c }).collect();
    let s = s.trim().trim_matches('.');
    if s.is_empty() { "photo".into() } else { s.to_string() }
}

/// The candidate `c` as the local file `path` (what the renderer can draw).
fn local_candidate(c: &ImportCandidate, path: &std::path::Path) -> ImportCandidate {
    ImportCandidate {
        path: path.to_string_lossy().to_string(),
        name: c.name.clone(),
        format: c.format.clone(),
        kind: c.kind,
        width: 0,
        height: 0,
        file_size: c.file_size,
        captured: c.captured.clone(),
        duplicate: None,
        existing: None,
        error: None,
        preview_only: None,
    }
}

/// Each frame: the listing's result, the loupe fetch's end, a run that ended without a report, and
/// a close a button asked for.
pub fn tick(app: &mut LightcraftApp, ctx: &egui::Context) {
    if app.camera_module.is_none() {
        return;
    }
    poll_listing(app, ctx);
    poll_preview(app);
    watch_run(app);
    if app.camera_module.as_ref().is_some_and(|m| m.close_requested) {
        close(app);
    }
}

fn poll_listing(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(m) = app.camera_module.as_mut() else { return };
    let Some(listing) = m.listing.as_ref() else { return };
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    match listing.rx.try_recv() {
        Ok(Ok(candidates)) => {
            m.listing = None;
            set_candidates(m, candidates);
            app.renderer.forget_imports();
        }
        Ok(Err(e)) => {
            m.listing = None;
            m.error = Some(e);
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => {}
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            m.listing = None;
            m.error = Some(crate::i18n::tr("The camera could not be read").to_string());
        }
    }
}

fn poll_preview(app: &mut LightcraftApp) {
    let Some(m) = app.camera_module.as_mut() else { return };
    let Some(p) = m.preview.as_mut() else { return };
    if !p.fetching {
        return;
    }
    let Some(state) = app.session.cameras.take_finished_of(&m.camera) else { return };
    p.fetching = false;
    let Some(c) = m.candidates.get(p.index) else { return };
    match state {
        State::Done { .. } => {
            let here = std::fs::metadata(&p.path).is_ok_and(|md| md.is_file() && md.len() == c.file_size);
            if here {
                p.local = Some(local_candidate(c, &p.path));
            } else {
                // the fetch that ended was another photo's: this one is asked for now
                let i = p.index;
                m.preview = None;
                show_photo(app, i);
            }
        }
        State::Failed(e) => p.failed = Some(e),
        State::Cancelled => {
            // stopped for another photo: ask for that one
            let i = p.index;
            m.preview = None;
            show_photo(app, i);
        }
        State::Running { .. } => p.fetching = true,
    }
}

/// A run whose copy and import are both gone without a report ended early (the import could not
/// start, say): say so, with what was counted.
fn watch_run(app: &mut LightcraftApp) {
    let Some(m) = app.camera_module.as_mut() else { return };
    let Some(run) = m.run.as_mut() else { return };
    if run.done {
        return;
    }
    let pending = app.camera_imports.iter().any(|p| p.camera == m.camera);
    if !pending && run.copied.is_some() && app.import.is_none() {
        run.done = true;
        if run.imported == 0 && run.failed == 0 && run.duplicates == 0 && run.error.is_none() {
            run.error = Some(crate::i18n::tr("The import did not run; the copies were not added").to_string());
        }
    }
}

/// The copy from `camera` ended (called by [`crate::camera_import::tick`] before it acts).
pub fn copy_ended(app: &mut LightcraftApp, camera: &str, state: &State) {
    let Some(m) = app.camera_module.as_mut() else { return };
    if m.camera != camera {
        return;
    }
    let Some(run) = m.run.as_mut() else { return };
    match state {
        State::Done { copied, skipped, targets } => {
            run.copied = Some((*copied, *skipped));
            if targets.is_empty() {
                run.done = true;
                run.error = Some(crate::i18n::tr("Nothing was copied from the camera").to_string());
            }
        }
        State::Failed(e) => {
            run.copied = Some((0, 0));
            run.done = true;
            run.error = Some(crate::i18n::tr_format!("Could not copy from the camera: {e}", e = e));
        }
        State::Cancelled => {
            run.copied = Some((0, 0));
            run.done = true;
            run.cancelled = true;
        }
        State::Running { .. } => {}
    }
}

/// The import of the copies ended (called by the import task as it finishes).
pub fn import_finished(app: &mut LightcraftApp, task: &crate::dialogs::import::ImportTask) {
    let Some(m) = app.camera_module.as_mut() else { return };
    let Some(run) = m.run.as_mut() else { return };
    if run.done || run.copied.is_none() {
        return;
    }
    run.imported = task.imported;
    run.restored = task.restored;
    run.duplicates = task.duplicates;
    run.failed = task.failed;
    run.cancelled |= task.cancelled;
    run.done = true;
}

/// Where the copy stands, for the options panel: `(bytes, total, files done, files total)` while it runs.
pub fn copy_progress(app: &LightcraftApp, camera: &str) -> Option<(u64, u64, usize, usize)> {
    match app.session.cameras.state_of(camera)? {
        State::Running { files_done, files_total, bytes, total } => Some((bytes, total, files_done, files_total)),
        _ => None,
    }
}

/// Where the import of the copies stands: `(done, total)` while it runs.
pub fn import_progress(app: &LightcraftApp, camera: &str) -> Option<(usize, usize)> {
    let importing = app.camera_imports.iter().any(|p| p.camera == camera && p.importing);
    let task = app.import.as_ref().filter(|_| importing)?;
    Some((task.done, task.total))
}

/// `ui.inspect` → `cameraModule`.
pub fn status(app: &LightcraftApp) -> Option<Value> {
    app.camera_module.as_ref().map(|m| m.status(app))
}

/// A short size: "24.3 MB".
pub fn size_text(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{bytes} B")
    }
}

/// A capture date as the lists show it: "2026-03-12 14:25".
pub fn date_text(captured: Option<&str>) -> String {
    match captured {
        Some(d) if d.len() >= 16 => format!("{} {}", d.get(..10).unwrap_or(""), d.get(11..16).unwrap_or("")),
        Some(d) => d.to_string(),
        None => crate::i18n::tr("No date").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(dates: &[Option<&str>]) -> CameraModule {
        let mut m = CameraModule::new("cam".into(), "Camera".into());
        let candidates = dates
            .iter()
            .enumerate()
            .map(|(i, d)| ImportCandidate {
                path: format!("ptp://cam/{i}/DCIM/IMG_{i}.CR3"),
                name: format!("IMG_{i}.CR3"),
                captured: d.map(str::to_string),
                ..Default::default()
            })
            .collect();
        set_candidates(&mut m, candidates);
        m
    }

    #[test]
    fn photos_group_by_capture_day_with_the_undated_last() {
        let m = module(&[Some("2026-03-12T14:25:01"), Some("2026-03-12T15:00:00"), Some("2026-03-13T08:00:00"), None]);
        assert_eq!(m.groups(), vec![("2026-03-12".to_string(), 0, 2), ("2026-03-13".to_string(), 2, 1), (String::new(), 3, 1)]);
    }

    #[test]
    fn a_shift_click_extends_from_the_last_click_and_skips_what_the_library_has() {
        let mut m = module(&[None; 5]);
        m.candidates[3].duplicate = Some("content".into());
        let candidates = m.candidates.clone();
        set_candidates(&mut m, candidates);
        assert_eq!(m.checked_count(), 4);
        m.click(0, false);
        m.click(4, true);
        assert_eq!(m.checked, vec![false, false, false, false, false]);
        m.click(2, false);
        m.click(0, true);
        assert_eq!(m.checked, vec![true, true, true, false, false]);
        m.click(3, false);
        assert!(!m.is_checked(3), "a duplicate stays unchecked");
        m.check_all(true);
        assert_eq!(m.checked, vec![true, true, true, false, true]);
    }

    #[test]
    fn the_options_become_import_parameters() {
        let mut m = module(&[None]);
        assert_eq!(m.organize(), ("date".to_string(), String::new()));
        m.by_date = false;
        assert_eq!(m.organize(), ("flat".to_string(), String::new()));
        m.subfolder = " Viagem/ ".into();
        assert_eq!(m.organize(), ("custom".to_string(), "Viagem".to_string()));
        m.by_date = true;
        assert_eq!(m.organize(), ("custom".to_string(), "Viagem/{date:%Y}/{date:%Y-%m-%d}".to_string()));
        assert_eq!(m.rename_template(), "");
        m.batch = "Lote".into();
        assert_eq!(m.rename_template(), "Lote-{seq:4}");
        m.batch = "{date}_{seq:3}".into();
        assert_eq!(m.rename_template(), "{date}_{seq:3}");
        assert_eq!(size_text(24_300_000), "24.3 MB");
        assert_eq!(date_text(Some("2026-03-12T14:25:01")), "2026-03-12 14:25");
    }

    #[test]
    fn a_run_sums_itself_up() {
        let mut run = Run { planned: 3, copied: Some((3, 0)), imported: 2, duplicates: 1, done: true, ..Default::default() };
        assert_eq!(run.summary(), "2 photos imported · 1 already in the library");
        run.cancelled = true;
        run.imported = 0;
        run.duplicates = 0;
        assert_eq!(run.summary(), "The copy was stopped; nothing was imported");
        run.error = Some("gone".into());
        assert_eq!(run.summary(), "gone");
    }
}
