//! The Import module's photo area: a toolbar (counts, the layout picker, Check All / Uncheck All,
//! the thumbnail size), then the camera's photos by capture day as a grid, a list or one photo at
//! a time. Every cell and row toggles its checkbox on a click (Shift extends the range), and opens
//! the photo on a double-click.

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use super::{CameraModule, LOUPE_SLOT, Layout, THUMB_MAX, THUMB_MIN};
use crate::LightcraftApp;
use crate::render::Slot;
use crate::theme::Tokens;
use crate::widgets::{icon_button, register, segmented, text_button};

const TOOLBAR_H: f32 = 36.0;
const HEADER_H: f32 = 34.0;
const GAP: f32 = 8.0;
const LABEL_H: f32 = 18.0;
const ROW_H: f32 = 26.0;

/// What the photo area asked for, done once the module is back in the app.
#[derive(Default)]
struct Asks {
    /// Show this photo in the loupe (fetching it from the camera).
    show: Option<usize>,
    /// List the camera again.
    retry: bool,
}

/// Draw the module's photo area (the window's centre).
pub fn show_center(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let Some(mut m) = app.camera_module.take() else { return };
    let t = Tokens::get(ui.ctx());
    register(ui, "cameraImport:view", ui.max_rect());
    crate::widgets::announce_region(ui, egui::accesskit::Role::Main, crate::i18n::tr("Import from camera"));
    ui.painter().rect_filled(ui.max_rect(), 0.0, t.grid_bg);
    let mut asks = Asks::default();
    toolbar(ui, &mut m, &t, &mut asks);
    ui.spacing_mut().item_spacing.y = 0.0;
    if let Some(e) = m.error.clone() {
        message(ui, &t, &crate::i18n::tr_format!("{name} could not be read", name = m.name), &e, Some(("cameraImportRetry", "Try Again")), &mut asks);
    } else if m.listing.is_some() {
        let text = crate::i18n::tr_format!("Reading {name}…", name = m.name);
        let r = ui.max_rect();
        crate::widgets::progress::paint_spinner(
            ui.painter(),
            Rect::from_center_size(r.center() - vec2(0.0, 24.0), vec2(22.0, 22.0)),
            t.accent,
            ui.input(|i| i.time),
        );
        ui.painter().text(r.center() + vec2(0.0, 8.0), Align2::CENTER_CENTER, text, t.font(13.0), t.text_dim);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    } else if m.candidates.is_empty() {
        message(
            ui,
            &t,
            crate::i18n::tr("No photos on the camera"),
            crate::i18n::tr("Photos the camera holds show up here as soon as it has some."),
            None,
            &mut asks,
        );
    } else {
        match m.layout {
            Layout::Grid => grid(app, ui, &mut m, &t, &mut asks),
            Layout::List => list(ui, &mut m, &t, &mut asks),
            Layout::Loupe => loupe(app, ui, &mut m, &t, &mut asks),
        }
    }
    app.camera_module = Some(m);
    if asks.retry {
        super::list(app);
    }
    if let Some(i) = asks.show {
        super::show_photo(app, i);
    }
}

/// A centred title and explanation, with an optional button.
fn message(ui: &mut egui::Ui, t: &Tokens, title: &str, text: &str, button: Option<(&str, &str)>, asks: &mut Asks) {
    let r = ui.max_rect();
    let p = ui.painter();
    p.text(r.center() - vec2(0.0, 30.0), Align2::CENTER_CENTER, title, t.semibold(16.0), t.text);
    let g = crate::widgets::one_line(p, text, t.font(12.5), t.text_dim, (r.width() - 80.0).max(100.0));
    let size = g.size();
    p.galley(r.center() - vec2(size.x / 2.0, 0.0), g, t.text_dim);
    if let Some((id, label)) = button {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_center_size(r.center() + vec2(0.0, 40.0), vec2(120.0, 24.0))));
        child.horizontal_centered(|ui| {
            if text_button(ui, id, label, false).clicked() {
                asks.retry = true;
            }
        });
    }
}

/// The row above the photos.
fn toolbar(ui: &mut egui::Ui, m: &mut CameraModule, t: &Tokens, asks: &mut Asks) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), TOOLBAR_H), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.chrome);
    ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, t.divider));
    register(ui, "cameraImport:toolbar", r);
    let inner = r.shrink2(vec2(12.0, 0.0));
    let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(inner));
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let n = m.candidates.len();
        let picked = m.checked_count();
        let counts = crate::i18n::tr_format!("{n} photo{} · {picked} selected", crate::i18n::plural_s(n), n = n, picked = picked);
        let name = ui.label(egui::RichText::new(&m.name).font(t.semibold(13.0)).color(t.text));
        register(ui, "label:cameraImportName", name.rect);
        let c = ui.label(egui::RichText::new(counts).font(t.font(11.5)).color(t.text_dim));
        register(ui, "label:cameraImportCounts", c.rect);
        // the right side, laid out from the right edge
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if text_button(ui, "cameraImportUncheckAll", "Uncheck All", false).clicked() {
                m.check_all(false);
            }
            if text_button(ui, "cameraImportCheckAll", "Check All", false).clicked() {
                m.check_all(true);
            }
            if m.layout == Layout::Grid {
                ui.add_space(8.0);
                let slider = ui.add(egui::Slider::new(&mut m.thumb_size, THUMB_MIN..=THUMB_MAX).show_value(false));
                register(ui, "slider:cameraImportThumbSize", slider.rect);
                crate::widgets::announce(&slider, crate::i18n::tr("Thumbnail size"));
                ui.add_space(8.0);
            }
            let items: Vec<(&str, &str)> = Layout::ALL.iter().map(|l| (l.label(), l.id())).collect();
            let active = Layout::ALL.iter().position(|l| *l == m.layout);
            ui.allocate_ui_with_layout(vec2(200.0, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if let Some(i) = segmented(ui, "cameraImportLayout", &items, active, 3)
                    && let Some(l) = Layout::ALL.get(i).copied()
                {
                    m.layout = l;
                    if l == Layout::Loupe {
                        asks.show = Some(m.current);
                    }
                }
            });
        });
    });
}

/// The grid: thumbnails by capture day.
fn grid(app: &mut LightcraftApp, ui: &mut egui::Ui, m: &mut CameraModule, t: &Tokens, asks: &mut Asks) {
    let cell = m.thumb_size.clamp(THUMB_MIN, THUMB_MAX);
    let avail = (ui.available_width() - 24.0).max(cell);
    let cols = ((avail + GAP) / (cell + GAP)).floor().max(1.0) as usize;
    let mut headers = Vec::new();
    let mut cells = Vec::with_capacity(m.candidates.len());
    let mut y = 8.0;
    for (key, start, count) in m.groups() {
        headers.push((key, Rect::from_min_size(pos2(12.0, y), vec2(avail, HEADER_H)), start, count));
        y += HEADER_H;
        for k in 0..count {
            let (c, r) = (k % cols, k / cols);
            let rect = Rect::from_min_size(pos2(12.0 + c as f32 * (cell + GAP), y + r as f32 * (cell + LABEL_H + GAP)), vec2(cell, cell + LABEL_H));
            cells.push((start + k, rect));
        }
        y += count.div_ceil(cols) as f32 * (cell + LABEL_H + GAP) + 8.0;
    }
    let mut visible = std::collections::HashSet::from([LOUPE_SLOT]);
    egui::ScrollArea::vertical().id_salt("cameraImport-grid").auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
        let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), y), Sense::hover());
        let origin = area.min.to_vec2();
        for (key, local, start, count) in &headers {
            if local.intersects(viewport) {
                group_header(ui, m, t, key, *start, *count, local.translate(origin));
            }
        }
        for (i, local) in &cells {
            if !local.intersects(viewport.expand(cell)) {
                continue;
            }
            let Ok(slot) = u32::try_from(*i) else { continue };
            visible.insert(slot);
            grid_cell(app, ui, m, t, *i, local.translate(origin), asks);
        }
    });
    if !ui.is_sizing_pass() {
        app.renderer.retain_imports(&visible);
    }
}

/// The day's name for a group key ("2026-03-12"; "" for photos without a date).
fn day_label(key: &str) -> String {
    if key.is_empty() { crate::i18n::tr("No capture date").to_string() } else { crate::i18n::date_group_label(key, false) }
}

/// A day's header: its name and count; a click checks the whole day (or unchecks it when it is all checked).
fn group_header(ui: &mut egui::Ui, m: &mut CameraModule, t: &Tokens, key: &str, start: usize, count: usize, r: Rect) {
    let resp = ui.interact(r, egui::Id::unique(("cameraImport-group", start)), Sense::click());
    let label = day_label(key);
    let n = crate::i18n::tr_format!("· {n} photo{}", crate::i18n::plural_s(count), n = count);
    crate::widgets::announce(&resp, format!("{label} {n}"));
    register(ui, format!("group:cameraImport:{}", if key.is_empty() { "unknown" } else { key }), r);
    let p = ui.painter();
    let g = p.layout_no_wrap(label, t.semibold(14.0), if resp.hovered() { t.text } else { t.text_label });
    let x = r.left() + 4.0;
    let gw = g.size().x;
    p.galley(pos2(x, r.center().y - g.size().y / 2.0), g, t.text);
    p.text(pos2(x + gw + 10.0, r.center().y), Align2::LEFT_CENTER, n, t.font(11.5), t.text_dim);
    if resp.clicked() {
        let range = start..(start + count).min(m.candidates.len());
        let all_on = range.clone().filter(|i| m.importable(*i)).all(|i| m.is_checked(i));
        for i in range {
            if m.importable(i)
                && let Some(c) = m.checked.get_mut(i)
            {
                *c = !all_on;
            }
        }
    }
}

/// The thumbnail bucket (pixels) for a cell `size` points wide.
fn thumb_edge(size: f32, pixels_per_point: f32) -> usize {
    let scale = if pixels_per_point.is_finite() && pixels_per_point > 0.0 { pixels_per_point } else { 1.0 };
    let pixels = (size - 8.0) * scale;
    [192, 384, 512, 768, 1024].into_iter().find(|edge| *edge as f32 >= pixels).unwrap_or(1024)
}

/// The checkbox at a cell's or row's corner.
fn checkbox(p: &egui::Painter, cb: Rect, on: bool, t: &Tokens) {
    p.rect(cb, 3.0, if on { t.accent } else { Color32::from_black_alpha(150) }, Stroke::new(1.0, Color32::from_gray(200)), StrokeKind::Inside);
    if on {
        p.line_segment([cb.left_center() + vec2(3.5, 0.5), cb.center_bottom() + vec2(-1.0, -4.0)], Stroke::new(2.0, Color32::WHITE));
        p.line_segment([cb.center_bottom() + vec2(-1.0, -4.0), cb.right_top() + vec2(-3.5, 4.0)], Stroke::new(2.0, Color32::WHITE));
    }
}

/// Why a candidate cannot be imported, for its badge.
fn badge_of(c: &lightcraft_engine::import::ImportCandidate) -> Option<&'static str> {
    match (&c.duplicate, &c.error) {
        (Some(_), _) => Some("In library"),
        (None, Some(_)) => Some("Unreadable"),
        _ => None,
    }
}

/// Draw the thumbnail texture of `slot` fitted in `img`, if it has arrived.
fn paint_thumb(app: &LightcraftApp, p: &egui::Painter, slot: Slot, img: Rect, tint: Color32) -> bool {
    let Some(tex) = app.renderer.textures.get(&slot) else { return false };
    let [tw, th] = tex.size;
    let s = ((img.width() - 8.0) / tw.max(1) as f32).min((img.height() - 8.0) / th.max(1) as f32);
    let fit = Rect::from_center_size(img.center(), vec2(tw as f32 * s, th as f32 * s));
    p.image(tex.tex.id(), fit, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
    true
}

/// One grid cell: the thumbnail (asked of the camera), the checkbox, a badge and the name.
fn grid_cell(app: &mut LightcraftApp, ui: &mut egui::Ui, m: &mut CameraModule, t: &Tokens, i: usize, rect: Rect, asks: &mut Asks) {
    let Some(c) = m.candidates.get(i).cloned() else { return };
    let Ok(slot_index) = u32::try_from(i) else { return };
    let ok = m.importable(i);
    let img = Rect::from_min_size(rect.min, vec2(rect.width(), rect.width()));
    let resp = ui.interact(img, egui::Id::unique(("cameraImport-cell", i)), Sense::click());
    crate::widgets::announce_selected(&resp, c.name.clone(), m.is_checked(i));
    register(ui, format!("cameraImport:cell:{i}"), img);
    let p = ui.painter();
    p.rect_filled(img, 4.0, t.skeleton);
    let edge = thumb_edge(img.width(), ui.ctx().pixels_per_point());
    app.renderer.import_preview(&mut app.session, &c, slot_index, edge);
    let tint = if ok { Color32::WHITE } else { Color32::from_gray(110) };
    paint_thumb(app, p, Slot::Import(slot_index), img, tint);
    let on = m.is_checked(i);
    if on {
        p.rect_stroke(img, 4.0, Stroke::new(2.0, t.accent), StrokeKind::Inside);
    } else if resp.hovered() {
        p.rect_stroke(img, 4.0, Stroke::new(1.0, t.text_dim), StrokeKind::Inside);
    }
    let cb = Rect::from_min_size(img.min + vec2(6.0, 6.0), vec2(16.0, 16.0));
    checkbox(p, cb, on, t);
    if let Some(b) = badge_of(&c) {
        let g = p.layout_no_wrap(crate::i18n::tr(b).to_string(), t.semibold(10.0), Color32::WHITE);
        let size = g.size() + vec2(8.0, 4.0);
        let br = Rect::from_min_size(pos2(img.right() - size.x - 6.0, img.top() + 6.0), size);
        p.rect_filled(br, 3.0, Color32::from_rgba_unmultiplied(170, 60, 50, 220));
        p.galley(br.min + vec2(4.0, 2.0), g, Color32::WHITE);
    }
    let color = if ok { t.text_label } else { t.text_dim };
    let name = crate::widgets::one_line(p, &c.name, t.font(10.5), color, (rect.width() - 4.0).max(1.0));
    p.galley(pos2(rect.left() + 2.0, img.bottom() + 3.0), name, color);
    let tip = format!("{}\n{} · {}", c.name, super::date_text(c.captured.as_deref()), super::size_text(c.file_size));
    let resp = resp.on_hover_text(tip);
    if resp.double_clicked() {
        m.layout = Layout::Loupe;
        asks.show = Some(i);
    } else if resp.clicked() && ok {
        let shift = ui.input(|input| input.modifiers.shift);
        m.click(i, shift);
    }
}

/// The list: one row per photo with name, date, size and format, under its day.
fn list(ui: &mut egui::Ui, m: &mut CameraModule, t: &Tokens, asks: &mut Asks) {
    let w = ui.available_width();
    // column titles
    let (hr, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
    ui.painter().rect_filled(hr, 0.0, t.chrome.gamma_multiply(0.97));
    ui.painter().hline(hr.x_range(), hr.bottom() - 0.5, Stroke::new(1.0, t.divider));
    let cols = columns(w);
    let p = ui.painter();
    for (x, label, align) in [
        (cols.name, "Name", Align2::LEFT_CENTER),
        (cols.date, "Date", Align2::LEFT_CENTER),
        (cols.size, "Size", Align2::RIGHT_CENTER),
        (cols.format, "Format", Align2::RIGHT_CENTER),
    ] {
        p.text(pos2(hr.left() + x, hr.center().y), align, crate::i18n::tr(label), t.semibold(11.0), t.text_dim);
    }
    let mut rows = Vec::with_capacity(m.candidates.len());
    let mut headers = Vec::new();
    let mut y = 4.0;
    for (key, start, count) in m.groups() {
        headers.push((key, Rect::from_min_size(pos2(12.0, y), vec2(w - 24.0, HEADER_H - 4.0)), start, count));
        y += HEADER_H - 4.0;
        for k in 0..count {
            rows.push((start + k, Rect::from_min_size(pos2(0.0, y), vec2(w, ROW_H))));
            y += ROW_H;
        }
        y += 6.0;
    }
    egui::ScrollArea::vertical().id_salt("cameraImport-list").auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
        let (area, _) = ui.allocate_exact_size(vec2(w, y), Sense::hover());
        let origin = area.min.to_vec2();
        for (key, local, start, count) in &headers {
            if local.intersects(viewport) {
                group_header(ui, m, t, key, *start, *count, local.translate(origin));
            }
        }
        for (i, local) in &rows {
            if local.intersects(viewport) {
                list_row(ui, m, t, *i, local.translate(origin), &cols, asks);
            }
        }
    });
}

/// Where the list's columns start (name, date) or end (size, format), from the left edge.
struct Columns {
    name: f32,
    date: f32,
    size: f32,
    format: f32,
}

fn columns(w: f32) -> Columns {
    let format = w - 16.0;
    let size = format - 70.0;
    let date = (size - 220.0).max(200.0);
    Columns { name: 40.0, date, size, format }
}

fn list_row(ui: &mut egui::Ui, m: &mut CameraModule, t: &Tokens, i: usize, r: Rect, cols: &Columns, asks: &mut Asks) {
    let Some(c) = m.candidates.get(i).cloned() else { return };
    let ok = m.importable(i);
    let resp = ui.interact(r, egui::Id::unique(("cameraImport-row", i)), Sense::click());
    crate::widgets::announce_selected(&resp, c.name.clone(), m.is_checked(i));
    register(ui, format!("cameraImport:row:{i}"), r);
    let on = m.is_checked(i);
    let p = ui.painter();
    if on {
        p.rect_filled(r.shrink2(vec2(8.0, 1.0)), 4.0, t.accent.gamma_multiply(0.18));
    } else if resp.hovered() {
        p.rect_filled(r.shrink2(vec2(8.0, 1.0)), 4.0, t.hover.gamma_multiply(0.5));
    }
    checkbox(p, Rect::from_center_size(pos2(r.left() + 22.0, r.center().y), vec2(16.0, 16.0)), on, t);
    let color = if ok { t.text } else { t.text_dim };
    let name_w = (cols.date - cols.name - 12.0).max(40.0);
    let name = crate::widgets::one_line(p, &c.name, t.font(12.0), color, name_w);
    p.galley(pos2(r.left() + cols.name, r.center().y - name.size().y / 2.0), name, color);
    if let Some(b) = badge_of(&c) {
        let bx = r.left() + cols.name + name_w.min(p.layout_no_wrap(c.name.clone(), t.font(12.0), color).size().x) + 8.0;
        p.text(pos2(bx, r.center().y), Align2::LEFT_CENTER, crate::i18n::tr(b), t.semibold(10.0), t.reject);
    }
    p.text(pos2(r.left() + cols.date, r.center().y), Align2::LEFT_CENTER, super::date_text(c.captured.as_deref()), t.font(11.5), t.text_label);
    p.text(pos2(r.left() + cols.size, r.center().y), Align2::RIGHT_CENTER, super::size_text(c.file_size), t.font(11.5), t.text_label);
    p.text(pos2(r.left() + cols.format, r.center().y), Align2::RIGHT_CENTER, &c.format, t.font(11.5), t.text_label);
    if resp.double_clicked() {
        m.layout = Layout::Loupe;
        asks.show = Some(i);
    } else if resp.clicked() && ok {
        let shift = ui.input(|input| input.modifiers.shift);
        m.click(i, shift);
    }
}

/// One photo at a time: previous / next, its facts, the checkbox, and the full photo fetched from
/// the camera (the grid's thumbnail stands in until it is here).
fn loupe(app: &mut LightcraftApp, ui: &mut egui::Ui, m: &mut CameraModule, t: &Tokens, asks: &mut Asks) {
    let n = m.candidates.len();
    m.current = m.current.min(n.saturating_sub(1));
    let i = m.current;
    let Some(c) = m.candidates.get(i).cloned() else { return };
    // the strip above the photo
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    let mut strip = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(12.0, 0.0))));
    let mut go: Option<usize> = None;
    strip.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if icon_button(ui, "cameraImportPrev", crate::icons::Icon::ChevronLeft, vec2(26.0, 26.0), false, i > 0, "Previous photo").clicked() {
            go = Some(i.saturating_sub(1));
        }
        if icon_button(ui, "cameraImportNext", crate::icons::Icon::ChevronRight, vec2(26.0, 26.0), false, i + 1 < n, "Next photo").clicked() {
            go = Some(i + 1);
        }
        ui.add_space(6.0);
        let caption = crate::i18n::tr_format!(
            "{name} · {k} of {n} · {date} · {size} · {format}",
            name = c.name,
            k = i + 1,
            n = n,
            date = super::date_text(c.captured.as_deref()),
            size = super::size_text(c.file_size),
            format = c.format
        );
        let r = ui.label(egui::RichText::new(caption).font(t.font(12.0)).color(t.text_label));
        register(ui, "label:cameraImportCaption", r.rect);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let on = m.is_checked(i);
            let label = if on { "✓ Selected" } else { "Select" };
            let enabled = m.importable(i);
            let r = ui.add_enabled_ui(enabled, |ui| text_button(ui, "cameraImportInclude", label, on)).inner;
            if r.clicked() && enabled {
                m.click(i, false);
            }
        });
    });
    // arrow keys step through the photos while nothing has the keyboard
    let free = ui.ctx().memory(|mem| mem.focused().is_none());
    if free {
        let (left, right) = ui.input(|input| (input.key_pressed(egui::Key::ArrowLeft), input.key_pressed(egui::Key::ArrowRight)));
        if left && i > 0 {
            go = Some(i - 1);
        } else if right && i + 1 < n {
            go = Some(i + 1);
        }
    }
    // the photo
    let area = ui.available_rect_before_wrap();
    register(ui, "cameraImport:loupe", area);
    let p = ui.painter();
    p.rect_filled(area, 0.0, t.canvas);
    let img = area.shrink(12.0);
    // the texture of another photo must not stand for this one
    if m.loupe_of != Some(i) {
        let keep: std::collections::HashSet<u32> =
            app.renderer.textures.keys().filter_map(|s| if let Slot::Import(k) = s { (*k != LOUPE_SLOT).then_some(*k) } else { None }).collect();
        app.renderer.retain_imports(&keep);
        m.loupe_of = Some(i);
    }
    let preview = m.preview.as_ref().filter(|pv| pv.index == i);
    let mut drawn = false;
    if let Some(local) = preview.and_then(|pv| pv.local.clone()) {
        let ppp = ui.ctx().pixels_per_point();
        let edge = ((img.width().max(img.height())) * ppp).ceil() as usize;
        app.renderer.import_preview(&mut app.session, &local, LOUPE_SLOT, edge.clamp(64, 1024));
        drawn = paint_thumb(app, p, Slot::Import(LOUPE_SLOT), img, Color32::WHITE);
    }
    if !drawn {
        // the grid's thumbnail, dimmed, until the file is here
        if let Ok(slot) = u32::try_from(i) {
            let edge = thumb_edge(m.thumb_size, ui.ctx().pixels_per_point());
            app.renderer.import_preview(&mut app.session, &c, slot, edge);
            paint_thumb(app, p, Slot::Import(slot), img, Color32::from_gray(150));
        }
        let text = match preview.and_then(|pv| pv.failed.clone()) {
            Some(e) => e,
            None if preview.is_some_and(|pv| pv.fetching) => crate::i18n::tr("Loading the full photo from the camera…").to_string(),
            None if preview.is_none() => crate::i18n::tr("Loading the full photo from the camera…").to_string(),
            None => crate::i18n::tr("Rendering…").to_string(),
        };
        let g = crate::widgets::one_line(p, &text, t.font(12.0), t.text, (img.width() - 40.0).max(60.0));
        let size = g.size() + vec2(28.0, 12.0);
        let pill = Rect::from_center_size(pos2(img.center().x, img.bottom() - 24.0), size);
        p.rect_filled(pill, 8.0, Color32::from_black_alpha(170));
        p.galley(pill.min + vec2(14.0, 6.0), g, t.text);
        register(ui, "cameraImport:loading", pill);
        if preview.is_none_or(|pv| pv.fetching || pv.failed.is_none()) {
            crate::widgets::progress::paint_spinner(
                p,
                Rect::from_center_size(img.center(), vec2(24.0, 24.0)),
                t.accent,
                ui.input(|input| input.time),
            );
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }
        if preview.is_none() {
            asks.show = Some(i);
        }
    }
    if let Some(k) = go {
        asks.show = Some(k);
    }
}
