//! Import from a camera over USB (PTP). File → Import from Device on a camera opens the import
//! review on the camera's own listing (thumbnails from the camera, oldest first); Import then
//! copies only the photos that were picked into a staging folder (`library.cameraCopy`, shown in
//! the activity stack) and runs the usual import on those files, with the destination, folder
//! organisation and the other options the review was given. The staging folder goes when the
//! import has finished.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::dialogs::import::{ImportDialog, ImportTask};

/// An import waiting for its copy from a camera, then for the import of the copies to end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    /// The camera's id (`Device::camera`).
    pub camera: String,
    /// The camera's name, for the notices.
    pub name: String,
    /// The `library.import` parameters the review chose (mode copy, destination, album…).
    pub params: Value,
    /// Where the copies land before the import moves them to their destination.
    pub staging: PathBuf,
    /// The copy is done and the import of the copies has started: the staging folder goes when it ends.
    pub importing: bool,
}

/// Import what the review picked from a camera: copy those files first, import them when the
/// copy ends. `params` are the import's, as the review built them.
pub fn start(app: &mut LightcraftApp, d: &ImportDialog, camera: &str, params: Value) -> Result<Value, String> {
    if app.camera_imports.iter().any(|p| p.camera == camera) {
        return Err(crate::i18n::tr("That camera is still being copied").to_string());
    }
    let name = d.sources.first().cloned().unwrap_or_else(|| crate::i18n::tr("camera").to_string());
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let staging = lightcraft_engine::devices::camera_root(camera).join(format!("incoming-{stamp}"));
    let mut items = Vec::new();
    for (i, c) in d.candidates.iter().enumerate() {
        if !d.checked.get(i).copied().unwrap_or(false) || !d.importable(i) {
            continue;
        }
        let Some((_, handle, relative)) = lightcraft_engine::import::parse_ptp_path(&c.path) else { continue };
        let mut target = staging.clone();
        for part in relative.split('/').filter(|p| !p.is_empty() && *p != "..") {
            target.push(part);
        }
        items.push(json!({"handle": handle, "size": c.file_size, "name": c.name, "target": target.to_string_lossy()}));
    }
    if items.is_empty() {
        return Err("no photos selected".into());
    }
    let n = items.len();
    app.session.execute("library.cameraCopy", &json!({"camera": camera, "name": name, "items": items})).map_err(|e| e.to_string())?;
    app.camera_imports.push(Pending { camera: camera.to_string(), name, params, staging, importing: false });
    Ok(json!({"copying": n}))
}

/// Each frame: a copy that ended starts the import of its files (or says why it did not), and an
/// import that ended takes its staging folder with it.
pub fn tick(app: &mut LightcraftApp, ctx: &egui::Context) {
    if app.camera_imports.is_empty() {
        return;
    }
    use lightcraft_engine::camera_fetch::State;
    for (camera, state) in app.session.cameras.take_finished() {
        let Some(pos) = app.camera_imports.iter().position(|p| p.camera == camera && !p.importing) else { continue };
        match state {
            State::Done { targets, .. } if !targets.is_empty() => {
                if app.import.is_some() {
                    let name = app.camera_imports[pos].name.clone();
                    app.toast_error(
                        ctx,
                        crate::i18n::tr_format!("Copied from {name}, but another import is running; import the copies by hand", name = name),
                    );
                    app.camera_imports.remove(pos);
                    continue;
                }
                let pending = &mut app.camera_imports[pos];
                pending.importing = true;
                let params = pending.params.clone();
                let paths: Vec<String> = targets.iter().map(|t| t.to_string_lossy().to_string()).collect();
                let undo0 = app.session.undo.len();
                app.import = Some(ImportTask::new(paths, params, undo0, false));
                app.renderer.forget_imports();
            }
            State::Done { .. } => {
                let pending = app.camera_imports.remove(pos);
                app.toast(ctx, crate::i18n::tr_format!("Nothing was copied from {name}", name = pending.name));
                let _ = std::fs::remove_dir_all(&pending.staging);
            }
            State::Failed(why) => {
                let pending = app.camera_imports.remove(pos);
                app.toast_error(ctx, crate::i18n::tr_format!("Could not copy from {name}: {why}", name = pending.name, why = why));
                let _ = std::fs::remove_dir_all(&pending.staging);
            }
            State::Cancelled => {
                let pending = app.camera_imports.remove(pos);
                app.toast(ctx, crate::i18n::tr_format!("Copy from {name} stopped", name = pending.name));
                let _ = std::fs::remove_dir_all(&pending.staging);
            }
            State::Running { .. } => {}
        }
    }
    // the import of the copies ended: the staging folder has served
    if app.import.is_none() {
        let done: Vec<Pending> = {
            let (done, keep): (Vec<_>, Vec<_>) = app.camera_imports.drain(..).partition(|p| p.importing);
            app.camera_imports = keep;
            done
        };
        for p in done {
            let _ = std::fs::remove_dir_all(&p.staging);
        }
    }
}
