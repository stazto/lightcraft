//! Copying a camera's files over USB (PTP) into its device folder, so the import review can read
//! them like a memory card. One fetch per camera at a time, on a worker thread, listed in the
//! activity stack (cancellable) while it runs; the UI polls [`Fetches::take_finished`] and opens
//! the import review on the folder when a fetch ends well.
//!
//! [`mirror`] is the copy itself over a [`Source`] (the camera, or a fake in tests): files the
//! folder already holds at the same size are skipped, each file lands under its camera path
//! (`DCIM/100CANON/IMG_0001.CR3`) and is written beside its target first, so a stopped copy leaves
//! no half file where the import would find it.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;

use crate::activity::{Activity, Cancel, TaskHandle, Unit};

/// Where a fetch stands. The UI shows Running in the activity stack and acts on the rest once.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum State {
    Running {
        #[serde(rename = "filesDone")]
        files_done: usize,
        #[serde(rename = "filesTotal")]
        files_total: usize,
        bytes: u64,
        total: u64,
    },
    Done {
        /// The files written (or already in place), in the order asked.
        targets: Vec<PathBuf>,
        copied: usize,
        skipped: usize,
    },
    Failed(String),
    Cancelled,
}

/// What the copy did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub copied: usize,
    pub skipped: usize,
    pub bytes: u64,
    /// Every target that is there now (copied or already in place), in the order asked.
    pub targets: Vec<PathBuf>,
}

/// Where files come from: the camera, or a fake in tests.
pub trait Source {
    fn objects(&mut self) -> Result<Vec<lightcraft_ptp::Object>, lightcraft_ptp::Error>;
    fn download(
        &mut self,
        handle: u32,
        sink: &mut dyn Write,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), lightcraft_ptp::Error>;
    /// The camera's own small JPEG of the object.
    fn thumb(&mut self, handle: u32) -> Result<Vec<u8>, lightcraft_ptp::Error>;
}

#[cfg(not(target_arch = "wasm32"))]
impl Source for lightcraft_ptp::Camera {
    fn objects(&mut self) -> Result<Vec<lightcraft_ptp::Object>, lightcraft_ptp::Error> {
        lightcraft_ptp::Camera::objects(self)
    }
    fn download(
        &mut self,
        handle: u32,
        sink: &mut dyn Write,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), lightcraft_ptp::Error> {
        lightcraft_ptp::Camera::download(self, handle, sink, cancel, progress)
    }
    fn thumb(&mut self, handle: u32) -> Result<Vec<u8>, lightcraft_ptp::Error> {
        lightcraft_ptp::Camera::thumb(self, handle)
    }
}

/// One file to copy: the camera's object and where it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub handle: u32,
    pub size: u64,
    /// Its name on the camera (for the activity row and messages).
    pub name: String,
    pub target: PathBuf,
}

/// Progress of a copy: files done of total, bytes done of total, the file being copied.
pub type Progress<'a> = &'a mut dyn FnMut(usize, usize, u64, u64, &str);

/// A path component as the camera named it, made safe for this file system: no separators, no
/// `..`, never empty.
fn safe_component(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() { "_".to_string() } else { trimmed.to_string() }
}

/// Copy every file of `source` under `root`, skipping what is already there at the same size.
pub fn mirror(source: &mut dyn Source, root: &Path, cancel: &AtomicBool, progress: Progress<'_>) -> Result<Summary, String> {
    let objects = source.objects().map_err(|e| e.to_string())?;
    let items: Vec<Item> = objects
        .into_iter()
        .filter(|o| !o.is_folder() && !o.name.trim().is_empty())
        .map(|o| {
            let mut target = root.to_path_buf();
            for folder in &o.folders {
                target.push(safe_component(folder));
            }
            target.push(safe_component(&o.name));
            Item { handle: o.handle, size: o.size, name: o.name, target }
        })
        .collect();
    copy_items(source, &items, cancel, progress)
}

/// Copy `items` from `source`; a target already there at the same size is kept as it is. Each
/// file is written beside its target first, so a stopped copy leaves no half file.
pub fn copy_items(source: &mut dyn Source, items: &[Item], cancel: &AtomicBool, progress: Progress<'_>) -> Result<Summary, String> {
    let mut summary = Summary::default();
    let mut plan = Vec::new();
    for item in items {
        match std::fs::metadata(&item.target) {
            Ok(m) if m.is_file() && m.len() == item.size => {
                summary.skipped = summary.skipped.saturating_add(1);
                summary.targets.push(item.target.clone());
            }
            _ => plan.push(item),
        }
    }
    let total: u64 = plan.iter().map(|i| i.size).sum();
    let files_total = plan.len();
    let mut done_bytes = 0u64;
    for (i, item) in plan.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        progress(i, files_total, done_bytes, total, &item.name);
        let parent = item.target.parent().ok_or_else(|| format!("{} has no folder", item.target.display()))?;
        std::fs::create_dir_all(parent).map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        let file_name = item.target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| safe_component(&item.name));
        let part = parent.join(format!(".{file_name}.part"));
        let result = {
            let mut file = std::fs::File::create(&part).map_err(|e| format!("could not write {}: {e}", part.display()))?;
            let mut on_bytes = |written: u64| progress(i, files_total, done_bytes.saturating_add(written), total, &item.name);
            let r = source.download(item.handle, &mut file, cancel, &mut on_bytes);
            r.and_then(|()| file.flush().map_err(|e| lightcraft_ptp::Error::Io(e.to_string())))
        };
        if let Err(e) = result {
            let _ = std::fs::remove_file(&part);
            return Err(match e {
                lightcraft_ptp::Error::Cancelled => "stopped".into(),
                e => format!("{}: {e}", item.name),
            });
        }
        std::fs::rename(&part, &item.target).map_err(|e| format!("could not place {}: {e}", item.target.display()))?;
        done_bytes = done_bytes.saturating_add(item.size);
        summary.copied = summary.copied.saturating_add(1);
        summary.bytes = done_bytes;
        summary.targets.push(item.target.clone());
    }
    progress(files_total, files_total, total, total, "");
    Ok(summary)
}

struct Shared {
    state: Mutex<State>,
    cancel: Arc<AtomicBool>,
}

impl Shared {
    fn set(&self, s: State) {
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = s;
    }
    fn get(&self) -> State {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// The camera copies of one session, by camera id.
#[derive(Default)]
pub struct Fetches {
    jobs: HashMap<String, Arc<Shared>>,
}

impl Fetches {
    /// Start copying `camera` into `root` (its files land under `root/DCIM/…`). One fetch per
    /// camera at a time. The activity stack lists it as "Copying from <name>" until it ends.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start(&mut self, camera: &lightcraft_ptp::CameraInfo, root: PathBuf, activity: &Activity) -> Result<(), String> {
        if self.jobs.get(&camera.id).is_some_and(|j| matches!(j.get(), State::Running { .. })) {
            return Err("that camera is already being copied".into());
        }
        std::fs::create_dir_all(&root).map_err(|e| format!("could not create {}: {e}", root.display()))?;
        let cancel = Arc::new(AtomicBool::new(false));
        let task = activity.start("import", &format!("Copying from {}", camera.name), Cancel::Flag(cancel.clone()));
        task.set_unit(Unit::Bytes);
        task.detail(&camera.name);
        let shared =
            Arc::new(Shared { state: Mutex::new(State::Running { files_done: 0, files_total: 0, bytes: 0, total: 0 }), cancel: cancel.clone() });
        let worker = shared.clone();
        let id = camera.id.clone();
        let camera = camera.clone();
        let handle: TaskHandle = task.handle();
        std::thread::Builder::new()
            .name("camera-fetch".into())
            .spawn(move || {
                // the row goes when the thread ends, however it ends
                let _task = task;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&camera, &root, &cancel, &worker, &handle)));
                match outcome {
                    Ok(Ok(summary)) => worker.set(State::Done { targets: summary.targets, copied: summary.copied, skipped: summary.skipped }),
                    Ok(Err(e)) if e == "stopped" => worker.set(State::Cancelled),
                    Ok(Err(e)) => worker.set(State::Failed(e)),
                    Err(_) => worker.set(State::Failed("the copy stopped unexpectedly".into())),
                }
            })
            .map_err(|e| format!("could not start the copy: {e}"))?;
        self.jobs.insert(id, shared);
        Ok(())
    }

    /// Every fetch this session has started, by camera id.
    pub fn snapshot(&self) -> Vec<(String, State)> {
        let mut all: Vec<_> = self.jobs.iter().map(|(id, j)| (id.clone(), j.get())).collect();
        all.sort_by(|a, b| a.0.cmp(&b.0));
        all
    }

    pub fn running(&self) -> bool {
        self.jobs.values().any(|j| matches!(j.get(), State::Running { .. }))
    }

    /// The fetches that ended since the last call, removed from the list.
    pub fn take_finished(&mut self) -> Vec<(String, State)> {
        let ended: Vec<String> = self.jobs.iter().filter(|(_, j)| !matches!(j.get(), State::Running { .. })).map(|(id, _)| id.clone()).collect();
        let mut out: Vec<(String, State)> = ended.into_iter().filter_map(|id| self.jobs.remove(&id).map(|j| (id, j.get()))).collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Ask the copy of `camera` to stop. `true` if there was one running.
    pub fn cancel(&self, camera: &str) -> bool {
        match self.jobs.get(camera) {
            Some(j) if matches!(j.get(), State::Running { .. }) => {
                j.cancel.store(true, Ordering::Relaxed);
                true
            }
            _ => false,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run(camera: &lightcraft_ptp::CameraInfo, root: &Path, cancel: &AtomicBool, shared: &Shared, task: &TaskHandle) -> Result<Summary, String> {
    let mut cam = lightcraft_ptp::Camera::open(camera).map_err(|e| e.to_string())?;
    let mut progress = |files_done: usize, files_total: usize, bytes: u64, total: u64, name: &str| {
        task.progress(bytes, total);
        if !name.is_empty() {
            task.detail(name);
        }
        shared.set(State::Running { files_done, files_total, bytes, total });
    };
    mirror(&mut cam, root, cancel, &mut progress)
}

#[cfg(not(target_arch = "wasm32"))]
type CameraSession = Arc<Mutex<lightcraft_ptp::Camera>>;

#[derive(Default)]
struct CamerasInner {
    /// The open PTP session of each camera, kept while the app runs (opening takes a moment).
    #[cfg(not(target_arch = "wasm32"))]
    sessions: HashMap<String, CameraSession>,
    /// The copies started, by camera id.
    jobs: HashMap<String, Arc<Shared>>,
}

/// The cameras of one session (`Device::camera` ids): the review lists and draws thumbnails from
/// an open session, and the copy of what was picked runs on a worker that holds it. Cloning shares
/// the same cameras, so a job on another thread can use them.
#[derive(Clone, Default)]
pub struct Cameras {
    inner: Arc<Mutex<CamerasInner>>,
}

impl Cameras {
    fn inner(&self) -> std::sync::MutexGuard<'_, CamerasInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The session with camera `id`, opened now when there is none yet.
    #[cfg(not(target_arch = "wasm32"))]
    fn session(&self, id: &str) -> Result<CameraSession, String> {
        let mut inner = self.inner();
        if let Some(s) = inner.sessions.get(id) {
            return Ok(s.clone());
        }
        let info = crate::devices::camera_by_id(id).ok_or_else(|| format!("no camera {id} is plugged in"))?;
        let camera = lightcraft_ptp::Camera::open(&info).map_err(|e| e.to_string())?;
        let s = Arc::new(Mutex::new(camera));
        inner.sessions.insert(id.to_string(), s.clone());
        Ok(s)
    }

    /// Forget the session with `id` (the camera went away, or the session is stale).
    pub fn forget(&self, id: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        self.inner().sessions.remove(id);
        #[cfg(target_arch = "wasm32")]
        let _ = id;
    }

    /// Every object on camera `id` (files and folders).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn list(&self, id: &str) -> Result<Vec<lightcraft_ptp::Object>, String> {
        let session = self.session(id)?;
        let mut camera = session.lock().unwrap_or_else(PoisonError::into_inner);
        let r = lightcraft_ptp::Camera::objects(&mut camera);
        drop(camera);
        self.after(id, r)
    }

    /// The camera's thumbnail of object `handle` on camera `id` (a small JPEG).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn thumb(&self, id: &str, handle: u32) -> Result<Vec<u8>, String> {
        let session = self.session(id)?;
        let mut camera = session.lock().unwrap_or_else(PoisonError::into_inner);
        let r = lightcraft_ptp::Camera::thumb(&mut camera, handle);
        drop(camera);
        self.after(id, r)
    }

    /// A camera that went away is forgotten, so the next call opens it again when it is back.
    #[cfg(not(target_arch = "wasm32"))]
    fn after<T>(&self, id: &str, r: Result<T, lightcraft_ptp::Error>) -> Result<T, String> {
        if matches!(r, Err(lightcraft_ptp::Error::Gone)) {
            self.forget(id);
        }
        r.map_err(|e| e.to_string())
    }

    #[cfg(target_arch = "wasm32")]
    pub fn list(&self, _id: &str) -> Result<Vec<lightcraft_ptp::Object>, String> {
        Err("cameras over USB are not reachable from the browser".into())
    }

    #[cfg(target_arch = "wasm32")]
    pub fn thumb(&self, _id: &str, _handle: u32) -> Result<Vec<u8>, String> {
        Err("cameras over USB are not reachable from the browser".into())
    }

    /// Copy `items` from camera `id` on a worker thread. One copy per camera at a time. The
    /// activity stack lists it as "Copying from <name>" until it ends; [`Cameras::take_finished`]
    /// reports how.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start(&self, id: &str, name: &str, items: Vec<Item>, activity: &Activity) -> Result<(), String> {
        if self.inner().jobs.get(id).is_some_and(|j| matches!(j.get(), State::Running { .. })) {
            return Err("that camera is already being copied".into());
        }
        let session = self.session(id)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let task = activity.start("import", &format!("Copying from {name}"), Cancel::Flag(cancel.clone()));
        task.set_unit(Unit::Bytes);
        task.detail(name);
        task.progress(0, items.iter().map(|i| i.size).sum());
        let shared = Arc::new(Shared {
            state: Mutex::new(State::Running { files_done: 0, files_total: items.len(), bytes: 0, total: 0 }),
            cancel: cancel.clone(),
        });
        let worker = shared.clone();
        let handle: TaskHandle = task.handle();
        std::thread::Builder::new()
            .name("camera-copy".into())
            .spawn(move || {
                // the row goes when the thread ends, however it ends
                let _task = task;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut camera = session.lock().unwrap_or_else(PoisonError::into_inner);
                    let mut progress = |files_done: usize, files_total: usize, bytes: u64, total: u64, file: &str| {
                        handle.progress(bytes, total);
                        if !file.is_empty() {
                            handle.detail(file);
                        }
                        worker.set(State::Running { files_done, files_total, bytes, total });
                    };
                    copy_items(&mut *camera, &items, &cancel, &mut progress)
                }));
                match outcome {
                    Ok(Ok(summary)) => worker.set(State::Done { targets: summary.targets, copied: summary.copied, skipped: summary.skipped }),
                    Ok(Err(e)) if e == "stopped" => worker.set(State::Cancelled),
                    Ok(Err(e)) => worker.set(State::Failed(e)),
                    Err(_) => worker.set(State::Failed("the copy stopped unexpectedly".into())),
                }
            })
            .map_err(|e| format!("could not start the copy: {e}"))?;
        self.inner().jobs.insert(id.to_string(), shared);
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    pub fn start(&self, _id: &str, _name: &str, _items: Vec<Item>, _activity: &Activity) -> Result<(), String> {
        Err("cameras over USB are not reachable from the browser".into())
    }

    /// Every copy this session has started, by camera id.
    pub fn snapshot(&self) -> Vec<(String, State)> {
        let mut all: Vec<_> = self.inner().jobs.iter().map(|(id, j)| (id.clone(), j.get())).collect();
        all.sort_by(|a, b| a.0.cmp(&b.0));
        all
    }

    pub fn running(&self) -> bool {
        self.inner().jobs.values().any(|j| matches!(j.get(), State::Running { .. }))
    }

    /// The copies that ended since the last call, removed from the list.
    pub fn take_finished(&self) -> Vec<(String, State)> {
        let mut inner = self.inner();
        let ended: Vec<String> = inner.jobs.iter().filter(|(_, j)| !matches!(j.get(), State::Running { .. })).map(|(id, _)| id.clone()).collect();
        let mut out: Vec<(String, State)> = ended.into_iter().filter_map(|id| inner.jobs.remove(&id).map(|j| (id, j.get()))).collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Ask the copy from camera `id` to stop. `true` if one was running.
    pub fn cancel(&self, id: &str) -> bool {
        match self.inner().jobs.get(id) {
            Some(j) if matches!(j.get(), State::Running { .. }) => {
                j.cancel.store(true, Ordering::Relaxed);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_ptp::Object;

    /// A camera with two folders of files, each file its handle repeated `size` times.
    struct Fake {
        objects: Vec<Object>,
        downloads: usize,
    }

    fn file(handle: u32, folders: &[&str], name: &str, size: u64) -> Object {
        Object {
            handle,
            storage: 0x10001,
            parent: 0,
            format: 0x3801,
            size,
            name: name.into(),
            captured: Some("20260312T142501".into()),
            folders: folders.iter().map(|s| s.to_string()).collect(),
        }
    }

    impl Source for Fake {
        fn objects(&mut self) -> Result<Vec<Object>, lightcraft_ptp::Error> {
            Ok(self.objects.clone())
        }
        fn download(
            &mut self,
            handle: u32,
            sink: &mut dyn Write,
            cancel: &AtomicBool,
            progress: &mut dyn FnMut(u64),
        ) -> Result<(), lightcraft_ptp::Error> {
            self.downloads += 1;
            let o = self.objects.iter().find(|o| o.handle == handle).ok_or(lightcraft_ptp::Error::Response(0x2009))?;
            let bytes = vec![handle as u8; o.size as usize];
            for (i, chunk) in bytes.chunks(3).enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    return Err(lightcraft_ptp::Error::Cancelled);
                }
                sink.write_all(chunk).map_err(|e| lightcraft_ptp::Error::Io(e.to_string()))?;
                progress(((i + 1) * 3).min(bytes.len()) as u64);
            }
            Ok(())
        }
        fn thumb(&mut self, _handle: u32) -> Result<Vec<u8>, lightcraft_ptp::Error> {
            Err(lightcraft_ptp::Error::Response(0x2010))
        }
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lc-camera-fetch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn files_land_under_their_camera_folders_and_a_second_pass_skips_them() {
        let root = temp("mirror");
        let mut cam = Fake {
            objects: vec![
                Object { format: 0x3001, ..file(1, &[], "DCIM", 0) },
                Object { format: 0x3001, ..file(2, &["DCIM"], "100CANON", 0) },
                file(3, &["DCIM", "100CANON"], "IMG_0001.CR3", 10),
                file(4, &["DCIM", "100CANON"], "IMG_0001.JPG", 7),
                file(5, &["MISC"], "../evil.txt", 2),
            ],
            downloads: 0,
        };
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        let mut progress = |fd: usize, ft: usize, b: u64, t: u64, name: &str| seen.push((fd, ft, b, t, name.to_string()));
        let s = mirror(&mut cam, &root, &cancel, &mut progress).unwrap();
        assert_eq!((s.copied, s.skipped, s.bytes), (3, 0, 19));
        assert_eq!(std::fs::read(root.join("DCIM/100CANON/IMG_0001.CR3")).unwrap(), vec![3u8; 10]);
        assert_eq!(std::fs::read(root.join("DCIM/100CANON/IMG_0001.JPG")).unwrap().len(), 7);
        assert!(root.join("MISC/_evil.txt").is_file(), "a name that climbs out is kept inside its folder");
        assert!(!root.join("DCIM/100CANON/.IMG_0001.CR3.part").exists(), "no part file is left");
        assert_eq!(seen.last().map(|p| (p.0, p.1, p.2, p.3)), Some((3, 3, 19, 19)));
        assert!(seen.iter().any(|p| p.4 == "IMG_0001.CR3" && p.2 > 0 && p.2 < 19), "bytes advance within a file");

        // the same camera again: nothing to copy
        cam.downloads = 0;
        let mut progress = |_: usize, _: usize, _: u64, _: u64, _: &str| {};
        let s = mirror(&mut cam, &root, &cancel, &mut progress).unwrap();
        assert_eq!((s.copied, s.skipped, cam.downloads), (0, 3, 0));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_stopped_copy_leaves_no_half_file() {
        let root = temp("cancel");
        let mut cam = Fake { objects: vec![file(7, &["DCIM", "100CANON"], "IMG_0007.CR3", 30)], downloads: 0 };
        let cancel = AtomicBool::new(false);
        let mut progress = |_: usize, _: usize, b: u64, _: u64, _: &str| {
            if b >= 9 {
                cancel.store(true, Ordering::Relaxed);
            }
        };
        let r = mirror(&mut cam, &root, &cancel, &mut progress);
        assert_eq!(r, Err("stopped".into()));
        assert!(!root.join("DCIM/100CANON/IMG_0007.CR3").exists());
        assert!(std::fs::read_dir(root.join("DCIM/100CANON")).unwrap().next().is_none(), "the part file is gone");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_camera_that_refuses_a_file_names_it() {
        let root = temp("refuse");
        let mut cam = Fake { objects: vec![file(9, &["DCIM"], "IMG_0009.CR3", 4)], downloads: 0 };
        cam.objects[0].handle = 9;
        struct Refusing(Fake);
        impl Source for Refusing {
            fn objects(&mut self) -> Result<Vec<Object>, lightcraft_ptp::Error> {
                self.0.objects()
            }
            fn download(&mut self, _: u32, _: &mut dyn Write, _: &AtomicBool, _: &mut dyn FnMut(u64)) -> Result<(), lightcraft_ptp::Error> {
                Err(lightcraft_ptp::Error::Response(0x200F))
            }
            fn thumb(&mut self, h: u32) -> Result<Vec<u8>, lightcraft_ptp::Error> {
                self.0.thumb(h)
            }
        }
        let cancel = AtomicBool::new(false);
        let mut progress = |_: usize, _: usize, _: u64, _: u64, _: &str| {};
        let e = mirror(&mut Refusing(cam), &root, &cancel, &mut progress).unwrap_err();
        assert!(e.starts_with("IMG_0009.CR3: ") && e.contains("access denied"), "{e}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn safe_components_keep_names_inside_the_folder() {
        assert_eq!(safe_component("IMG_0001.CR3"), "IMG_0001.CR3");
        assert_eq!(safe_component(".."), "_");
        assert_eq!(safe_component("a/b\\c:d"), "a_b_c_d");
        assert_eq!(safe_component("   "), "_");
    }
}
