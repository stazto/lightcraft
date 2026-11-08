//! The Import module's right-hand panel: Import Selected / Import All, the destination (the
//! library's Originals or a chosen folder, a subfolder, by date or not), the batch name the copies
//! are renamed with, the album, the progress of the copy and of the import while they run and a
//! summary at the end, and Back to Library.

use egui::{Align2, Sense, vec2};

use super::CameraModule;
use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::progress::{Progress, fraction_of};
use crate::widgets::text_field::TextField;
use crate::widgets::{divider, register, text_button};

/// Draw the module's options (the Tools dock panel while the module is open).
pub fn show_options(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let Some(mut m) = app.camera_module.take() else { return };
    let t = Tokens::get(ui.ctx());
    register(ui, "panel:cameraImport", ui.max_rect());
    let mut start: Option<bool> = None;
    egui::ScrollArea::vertical().id_salt("cameraImport-options").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        crate::panels::right::header(ui, "Import");
        padded(ui, |ui| {
            ui.label(egui::RichText::new(&m.name).font(t.semibold(12.5)).color(t.text));
            let n = m.candidates.len();
            let picked = m.checked_count();
            ui.label(
                egui::RichText::new(crate::i18n::tr_format!(
                    "{n} photo{} on the camera · {picked} selected",
                    crate::i18n::plural_s(n),
                    n = n,
                    picked = picked
                ))
                .font(t.font(11.5))
                .color(t.text_dim),
            );
        });
        ui.add_space(6.0);
        let running = m.run.as_ref().is_some_and(|r| !r.done) || m.busy(app);
        padded(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let picked = m.checked_count();
            let all = m.importable_count();
            let selected = crate::i18n::tr_format!("Import Selected ({n})", n = picked);
            if primary_button(ui, "cameraImportSelected", &selected, !running && picked > 0, true, &t).clicked() {
                start = Some(false);
            }
            let every = crate::i18n::tr_format!("Import All ({n})", n = all);
            if primary_button(ui, "cameraImportAll", &every, !running && all > 0, false, &t).clicked() {
                start = Some(true);
            }
            if let Some(notice) = &m.notice {
                let r = ui.label(egui::RichText::new(notice).font(t.font(11.0)).color(t.reject));
                register(ui, "label:cameraImportNotice", r.rect);
            }
        });
        ui.add_space(8.0);
        divider(ui);
        section_title(ui, &t, "Destination");
        padded(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            destination(app, ui, &mut m, &t);
        });
        divider(ui);
        section_title(ui, &t, "File Naming");
        padded(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            naming(ui, &mut m, &t);
        });
        divider(ui);
        section_title(ui, &t, "Album");
        padded(ui, |ui| {
            album(app, ui, &mut m);
        });
        if m.run.is_some() {
            divider(ui);
            section_title(ui, &t, "Progress");
            padded(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                progress(app, ui, &mut m, &t);
            });
        }
        ui.add_space(10.0);
        divider(ui);
        padded(ui, |ui| {
            ui.add_space(4.0);
            if text_button(ui, "cameraImportClose", "Back to Library", false).clicked() {
                m.close_requested = true;
            }
            ui.add_space(8.0);
        });
    });
    app.camera_module = Some(m);
    if let Some(all) = start
        && let Err(e) = super::start_import(app, all)
        && let Some(m) = app.camera_module.as_mut()
    {
        m.notice = Some(e);
    }
}

fn padded(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 22, top: 6, bottom: 6 }).show(ui, add);
}

fn section_title(ui: &mut egui::Ui, t: &Tokens, title: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    ui.painter().text(egui::pos2(r.left() + 24.0, r.center().y + 2.0), Align2::LEFT_CENTER, crate::i18n::tr(title), t.semibold(13.0), t.text);
}

/// A full-width button: the accent one for the main action, a plain one beside it.
fn primary_button(ui: &mut egui::Ui, id: &str, label: &str, enabled: bool, accent: bool, t: &Tokens) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    register(ui, format!("button:{id}"), r);
    let p = ui.painter();
    let (fill, text) = match (accent, enabled) {
        (true, true) => (if resp.hovered() { t.accent.gamma_multiply(1.1) } else { t.accent }, t.canvas),
        (true, false) => (t.accent.gamma_multiply(0.35), t.text_disabled),
        (false, true) => (if resp.hovered() { t.hover } else { t.button }, t.text),
        (false, false) => (t.button, t.text_disabled),
    };
    p.rect(r, 7.0, fill, egui::Stroke::new(1.0, if accent { fill } else { t.button_border }), egui::StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, label, t.semibold(12.0), text);
    resp
}

/// A row with a radio dot.
fn radio_row(ui: &mut egui::Ui, id: &str, label: &str, on: bool, t: &Tokens) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    crate::widgets::announce_selected(&resp, crate::i18n::tr(label), on);
    register(ui, format!("radio:{id}"), r);
    let p = ui.painter();
    let c = egui::pos2(r.left() + 8.0, r.center().y);
    p.circle_stroke(c, 6.0, egui::Stroke::new(1.0, if on { t.accent } else { t.text_dim }));
    if on {
        p.circle_filled(c, 3.5, t.accent);
    }
    p.text(egui::pos2(r.left() + 22.0, r.center().y), Align2::LEFT_CENTER, crate::i18n::tr(label), t.font(12.0), t.text);
    resp
}

fn destination(app: &mut LightcraftApp, ui: &mut egui::Ui, m: &mut CameraModule, t: &Tokens) {
    let library = m.destination.trim().is_empty();
    if radio_row(ui, "cameraImportDest-library", "Library Originals", library, t).clicked() {
        m.destination.clear();
    }
    if radio_row(ui, "cameraImportDest-folder", "Another folder", !library, t).clicked() && library {
        // pick one at once; nothing picked keeps the library
        if let Some(pick) = app.services.pick_folder.as_mut()
            && let Some(d) = pick()
        {
            m.destination = d;
        }
    }
    if !library {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let w = (ui.available_width() - 80.0).max(60.0);
            TextField::singleline("field:cameraImportDestination", &mut m.destination).width(w).show(ui);
            if app.services.pick_folder.is_some()
                && text_button(ui, "cameraImportChoose", "Choose…", false).clicked()
                && let Some(pick) = app.services.pick_folder.as_mut()
                && let Some(d) = pick()
            {
                m.destination = d;
            }
        });
    }
    ui.add_space(2.0);
    ui.label(egui::RichText::new(crate::i18n::tr("Subfolder")).font(t.font(11.0)).color(t.text_dim));
    TextField::singleline("field:cameraImportSubfolder", &mut m.subfolder)
        .hint(crate::i18n::tr("none (created when given)"))
        .width(ui.available_width())
        .show(ui);
    let r = ui.checkbox(&mut m.by_date, crate::i18n::tr("Organize by date (year / day)"));
    register(ui, "check:cameraImportByDate", r.rect);
    if let Some(example) = example(m) {
        let r = ui.label(egui::RichText::new(crate::i18n::tr_format!("Example: {example}", example = example)).font(t.font(10.5)).color(t.text_dim));
        register(ui, "label:cameraImportExample", r.rect);
    }
}

/// Where the first checked photo would land, for the example line.
fn example(m: &CameraModule) -> Option<String> {
    let i = (0..m.candidates.len()).find(|i| m.is_checked(*i)).or_else(|| (!m.candidates.is_empty()).then_some(0))?;
    let c = m.candidates.get(i)?;
    let mut path = if m.destination.trim().is_empty() {
        "Originals".to_string()
    } else {
        std::path::Path::new(m.destination.trim())
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| m.destination.trim().to_string())
    };
    let sub = m.subfolder.trim().trim_matches(['/', '\\']);
    if !sub.is_empty() {
        path = format!("{path}/{sub}");
    }
    if m.by_date {
        let day = c.captured.as_deref().and_then(|d| d.get(..10)).unwrap_or("2026-01-01");
        let year = day.get(..4).unwrap_or("2026");
        path = format!("{path}/{year}/{day}");
    }
    let name = match m.rename_template().is_empty() {
        true => c.name.clone(),
        false => {
            let ext = std::path::Path::new(&c.name).extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
            let stem = m.batch.trim();
            if ext.is_empty() { format!("{stem}-0001") } else { format!("{stem}-0001.{ext}") }
        }
    };
    Some(format!("{path}/{name}"))
}

fn naming(ui: &mut egui::Ui, m: &mut CameraModule, t: &Tokens) {
    ui.label(egui::RichText::new(crate::i18n::tr("Batch name")).font(t.font(11.0)).color(t.text_dim));
    TextField::singleline("field:cameraImportBatch", &mut m.batch)
        .hint(crate::i18n::tr("keep the camera's names"))
        .width(ui.available_width())
        .show(ui);
    let template = m.rename_template();
    let text = if template.is_empty() {
        crate::i18n::tr("Files keep their names (IMG_0001.CR3).").to_string()
    } else {
        crate::i18n::tr_format!("Files are named {template} (0001, 0002…).", template = template)
    };
    let r = ui.label(egui::RichText::new(text).font(t.font(10.5)).color(t.text_dim));
    register(ui, "label:cameraImportRename", r.rect);
}

fn album(app: &mut LightcraftApp, ui: &mut egui::Ui, m: &mut CameraModule) {
    let mut albums = crate::widgets::album_picker::entries_from(&app.session.catalog, |_| None);
    albums.retain(|a| a.folder || !a.smart);
    m.album = m.album.filter(|id| albums.iter().any(|a| a.id == *id && !a.folder));
    let r = crate::widgets::album_picker::AlbumPicker::new("cameraImport-album", &mut m.album, &albums)
        .width(ui.available_width())
        .empty_text(crate::i18n::tr("None"))
        .show(ui);
    crate::widgets::name_combo(&r, crate::i18n::tr("Album"));
}

fn progress(app: &mut LightcraftApp, ui: &mut egui::Ui, m: &mut CameraModule, t: &Tokens) {
    let Some(run) = m.run.clone() else { return };
    if run.done {
        let r = ui.label(egui::RichText::new(run.summary()).font(t.font(11.5)).color(if run.error.is_some() { t.reject } else { t.text }));
        register(ui, "label:cameraImportSummary", r.rect);
        if let Some((copied, skipped)) = run.copied.filter(|_| run.error.is_none()) {
            let text = if skipped > 0 {
                crate::i18n::tr_format!(
                    "{copied} file{} copied from the camera, {skipped} already here",
                    crate::i18n::plural_s(copied),
                    copied = copied,
                    skipped = skipped
                )
            } else {
                crate::i18n::tr_format!("{copied} file{} copied from the camera", crate::i18n::plural_s(copied), copied = copied)
            };
            ui.label(egui::RichText::new(text).font(t.font(10.5)).color(t.text_dim));
        }
        if text_button(ui, "cameraImportDone", "OK", false).clicked() {
            m.run = None;
        }
        return;
    }
    if run.copied.is_none() {
        match super::copy_progress(app, &m.camera) {
            Some((bytes, total, files_done, files_total)) => {
                let label = crate::i18n::tr_format!(
                    "Copying {k} of {n} · {done} of {all}",
                    k = (files_done + 1).min(files_total.max(1)),
                    n = files_total.max(run.planned),
                    done = super::size_text(bytes),
                    all = super::size_text(total)
                );
                Progress::new(fraction_of(bytes, total)).label(label).widget("progress:cameraImportCopy").show(ui);
            }
            None => {
                Progress::new(None).label(crate::i18n::tr("Copying from the camera…")).widget("progress:cameraImportCopy").show(ui);
            }
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        return;
    }
    let (done, total) = super::import_progress(app, &m.camera).unwrap_or((0, run.planned));
    let label = crate::i18n::tr_format!("Importing {done} of {total}", done = done, total = total.max(1));
    Progress::new(fraction_of(done as u64, total.max(1) as u64)).label(label).widget("progress:cameraImportImport").show(ui);
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
}
