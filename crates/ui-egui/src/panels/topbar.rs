//! The top bar: sidebar toggle, back/forward, the identity plate, the module picker (Library,
//! Develop, Slideshow, Print, as Lightroom Classic arranges them) and the right-hand icons.

use egui::{Align2, Rect, Sense, pos2, vec2};
use serde_json::json;

use crate::LightcraftApp;
use crate::icons::{Icon, paint};
use crate::state::{RightPanel, ViewMode};
use crate::theme::Tokens;
use crate::widgets::{icon_button, register};

/// The product name shown on the identity plate.
pub const PRODUCT: &str = "Lightspot";

/// The modules of the picker: widget id, label and the command a click runs.
const MODULES: &[(&str, &str, &str)] = &[
    ("library", "Library", "view.photoGrid"),
    ("develop", "Develop", "view.detail"),
    ("slideshow", "Slideshow", "view.slideshow"),
    ("print", "Print", "dialog.print"),
];

/// The Import module's entry in the picker, there (and lit) while a camera's photos fill the window.
const IMPORT_MODULE: (&str, &str, &str) = ("import", "Import", "import.cameraClose");

/// The picker's modules right now: the fixed four, and Import while the Import module is open.
fn modules(app: &LightcraftApp) -> Vec<(&'static str, &'static str, &'static str)> {
    let mut v = MODULES.to_vec();
    if crate::camera_module::is_open(app) {
        v.push(IMPORT_MODULE);
    }
    v
}

/// Which module is lit: Import while the Import module is open, the slideshow while it runs,
/// Develop in the detail view, else Library.
fn active_module(app: &LightcraftApp) -> &'static str {
    if crate::camera_module::is_open(app) {
        "import"
    } else if app.ui.slideshow.is_some() {
        "slideshow"
    } else if app.ui.view == ViewMode::Detail {
        "develop"
    } else {
        "library"
    }
}

/// The bar doubles as the window's title bar: dragging its empty space moves the window and a
/// double-click acts as the system setting says. Made before the bar's widgets so they win the click.
fn window_handle(ui: &mut egui::Ui, bar: Rect) {
    register(ui, "region:titlebar", bar);
    let resp = craft_window::title_bar::empty_area(ui, bar);
    crate::widgets::announce(&resp, crate::i18n::tr("Title bar"));
}

/// Draw the top bar: the titlebar drag area, the in-window menus when there is no native menu
/// bar, sidebar and back/forward buttons, the identity plate, the module picker and the
/// right-side action icons.
pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    // room for the traffic lights; in full screen they are gone, and the sidebar button sits as far
    // from the left edge as the last icon from the right (12 margin + 4)
    let left = match (app.integrated_titlebar, app.window_is_fullscreen) {
        (true, false) => 78,
        (true, true) => 16,
        (false, _) => 10,
    };
    // A dialog fades the disabled bar's controls, and its fill with them. A sheet leaves the title bar's
    // color alone (the window buttons the OS draws over it stay lit), so the fill is painted unfaded
    // beneath, in a slot reserved before the bar's own shapes.
    let sheet = app.integrated_titlebar && !app.window_is_fullscreen && app.ui.dialog.is_some();
    let slot = sheet.then(|| ui.ctx().layer_painter(ui.layer_id()).add(egui::Shape::Noop));
    let fill = if sheet { egui::Color32::TRANSPARENT } else { t.chrome };
    let panel = egui::Panel::top("top_bar")
        .exact_size(t.top_bar_h)
        .frame(egui::Frame::NONE.fill(fill).inner_margin(egui::Margin { left, right: 12, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            crate::widgets::announce_region(ui, egui::accesskit::Role::Toolbar, crate::i18n::tr("Top bar"));
            let full = ui.max_rect();
            // the whole bar, its margins included
            let bar = Rect::from_min_max(pos2(full.left() - left as f32, full.top()), pos2(full.right() + 12.0, full.bottom()));
            if app.integrated_titlebar {
                window_handle(ui, bar);
            }
            let x = right_icons(app, ui, &t, full);
            let x = appearance_button(app, ui, &t, full, x);
            let x = filter_button(app, ui, &t, full, x);
            // the picker ends 20 points before the icons; what is left of it goes to the menus
            let modules_w = module_picker_width(app, ui, &t);
            let modules_right = x - 14.0 - 20.0;
            let modules_left = modules_right - modules_w;
            nav_buttons(app, ui, modules_left);
            module_picker(app, ui, &t, full, modules_left, modules_w);
        });
    if let Some(slot) = slot {
        ui.ctx().layer_painter(ui.layer_id()).set(slot, egui::Shape::rect_filled(panel.response.rect, 0.0, t.chrome));
    }
}

/// Sidebar toggle, back/forward and then the identity plate with a native menu bar, or the
/// in-window menus without one. `limit` is where the module picker starts.
fn nav_buttons(app: &mut LightcraftApp, ui: &mut egui::Ui, limit: f32) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if icon_button(ui, "sidebar", Icon::Sidebar, vec2(26.0, 26.0), app.ui.left_panel, true, crate::i18n::tr("Show/hide My Photos panel"))
            .clicked()
        {
            let _ = app.run("view.leftPanel", json!({}));
        }
        ui.add_space(6.0);
        if icon_button(ui, "back", Icon::Back, vec2(26.0, 26.0), false, true, crate::i18n::tr("Back")).clicked() {
            let _ = app.run("view.back", json!({}));
        }
        icon_button(ui, "forward", Icon::Forward, vec2(26.0, 26.0), false, false, crate::i18n::tr("Forward"));
        if app.native_menu {
            ui.add_space(10.0);
            identity_plate(ui, &t, limit);
        } else {
            // no native menu bar (web, Windows, Linux): menus in the top bar
            ui.add_space(10.0);
            let room = limit - ui.cursor().left() - 16.0;
            crate::menubar::show_in_window(app, ui, room);
        }
    });
}

/// The mark (a rounded navy tile with "Ls" in cyan, drawn here) and the product name beside it,
/// as Lightroom's identity plate sits at the left of its top bar. Skipped when there is no room.
fn identity_plate(ui: &mut egui::Ui, t: &Tokens, limit: f32) {
    let name = ui.painter().layout_no_wrap(PRODUCT.into(), t.semibold(13.0), t.text);
    let w = 22.0 + 8.0 + name.size().x;
    if ui.cursor().left() + w > limit - 24.0 {
        return;
    }
    let (r, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::hover());
    crate::widgets::announce(&resp, PRODUCT);
    register(ui, "brand:identity", r);
    let p = ui.painter();
    let tile = Rect::from_center_size(pos2(r.left() + 11.0, r.center().y), vec2(22.0, 22.0));
    p.rect_filled(tile, 6.0, egui::Color32::from_rgb(0x02, 0x17, 0x3b));
    p.text(tile.center() + vec2(0.0, -0.5), Align2::CENTER_CENTER, "Ls", t.semibold(12.0), egui::Color32::from_rgb(0x2d, 0xc0, 0xfe));
    p.galley(pos2(tile.right() + 8.0, r.center().y - name.size().y / 2.0), name, t.text);
}

/// The width the module picker takes: its labels, the gaps and the dividers between them.
fn module_picker_width(app: &LightcraftApp, ui: &egui::Ui, t: &Tokens) -> f32 {
    let mut w = 0.0;
    let modules = modules(app);
    for (i, (_, label, _)) in modules.iter().enumerate() {
        let g = ui.painter().layout_no_wrap(crate::i18n::tr(label).into(), t.semibold(13.0), t.text);
        w += g.size().x + 20.0;
        if i + 1 < modules.len() {
            w += 9.0;
        }
    }
    w
}

/// Lightroom's module picker: the modules' names in a row, the current one lit, thin dividers
/// between them. A click goes to that module.
fn module_picker(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, full: Rect, left: f32, width: f32) {
    let row = Rect::from_min_size(pos2(left, full.top()), vec2(width, full.height()));
    register(ui, "modules", row);
    let active = active_module(app);
    let mut x = row.left();
    let mut run = None;
    let modules = modules(app);
    for (i, (id, label, cmd)) in modules.iter().enumerate() {
        let text = crate::i18n::tr(label);
        // laid out in the placeholder colour, so the colour given when it is painted is the one shown
        let g = ui.painter().layout_no_wrap(text.into(), t.semibold(13.0), egui::Color32::PLACEHOLDER);
        let r = Rect::from_min_size(pos2(x, row.top() + 6.0), vec2(g.size().x + 20.0, row.height() - 12.0));
        let resp = ui.interact(r, egui::Id::unique(("module", *id)), Sense::click());
        crate::widgets::announce_selected(&resp, text, *id == active);
        register(ui, format!("module:{id}"), r);
        let on = *id == active;
        let colour = if on {
            t.text
        } else if resp.hovered() {
            t.text_label
        } else {
            t.text_dim
        };
        if resp.hovered() && !on {
            ui.painter().rect_filled(r, 6.0, t.hover.gamma_multiply(0.5));
        }
        ui.painter().galley(pos2(r.left() + 10.0, r.center().y - g.size().y / 2.0), g, colour);
        if resp.clicked() && !on {
            run = Some((*id, *cmd));
        }
        x = r.right();
        if i + 1 < modules.len() {
            let div = Rect::from_center_size(pos2(x + 4.5, row.center().y), vec2(1.0, 12.0));
            ui.painter().rect_filled(div, 0.0, t.text_disabled);
            x += 9.0;
        }
    }
    if let Some((id, cmd)) = run {
        // leaving for another module closes the Import module (its copy goes on in the activity stack)
        if id != "import" && crate::camera_module::is_open(app) {
            crate::camera_module::close(app);
        }
        if let Err(e) = app.run(cmd, json!({})) {
            app.toast_error(ui.ctx(), e);
        } else if id == "develop" && !app.ui.right.is_edit_tool() {
            // Develop shows the photo with its sliders
            let _ = app.run("panel.edit", json!({}));
        } else if id == "develop" && app.ui.right == RightPanel::None {
            let _ = app.run("panel.edit", json!({}));
        }
    }
}

/// The filter icon at `x`, with the active-filter badge; returns the x of the next control.
fn filter_button(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, full: Rect, x: f32) -> f32 {
    let fr = Rect::from_center_size(pos2(x, full.center().y), vec2(24.0, 24.0));
    let fresp = ui.interact(fr, egui::Id::unique("filter-btn"), Sense::click());
    crate::widgets::announce(&fresp, crate::i18n::tr("Filters"));
    register(ui, "icon:filter", fr);
    let filtering = app.session.filter != Default::default() || app.ui.filter_bar;
    paint(
        ui.painter(),
        fr.shrink(5.0),
        Icon::Filter,
        if filtering {
            t.accent
        } else if fresp.hovered() {
            t.text
        } else {
            t.icon
        },
    );
    // badge: how many filters are on, even with the filter bar closed
    let active = lightcraft_engine::filter_chips(&app.session.filter, &app.session.catalog).len();
    if active > 0 {
        let c = fr.right_top() + vec2(-3.0, 6.0);
        ui.painter().circle_filled(c, 6.5, t.accent);
        ui.painter().text(c, Align2::CENTER_CENTER, active.to_string(), t.semibold(9.0), t.canvas);
    }
    let fresp = fresp.on_hover_text(if active > 0 {
        crate::i18n::tr_format!("Filter bar — {active} active filter{}", if active == 1 { "" } else { "s" }, active = active)
    } else {
        crate::i18n::tr("Filter bar").into()
    });
    if fresp.clicked() {
        let _ = app.run("view.filterBar", json!({}));
    }
    x - 34.0
}

/// The right-side action icons; returns the x where the next icon goes.
fn right_icons(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, full: Rect) -> f32 {
    let mut x = full.right() - 16.0;
    // saving is failing: the cloud icon turns into a warning until a save succeeds
    let unsaved = app.session.unsaved().map(|(n, e)| unsaved_tip(n, e));
    let cloud_tip = unsaved.as_deref().unwrap_or("Local library — no cloud account needed");
    // Import Photos… beside Export, always one click away, with the key it is bound to now
    let mac = ui.ctx().os() == egui::os::OperatingSystem::Mac;
    let import_tip = match crate::shortcuts::shortcut_of(&app.ui.settings.keymap, "file.addPhotos") {
        Some(sc) => format!("{} ({})", crate::i18n::tr("Import Photos…"), crate::menubar::shortcut_text(sc, mac)),
        None => crate::i18n::tr("Import Photos…").to_string(),
    };
    for (id, icon, tip, cmd) in [
        ("cloud", Icon::Cloud, cloud_tip, ""),
        ("help", Icon::Help, "Keyboard shortcuts", "app.shortcuts"),
        ("share", Icon::Share, "Export", "dialog.export"),
        ("import", Icon::Plus, import_tip.as_str(), "file.addPhotos"),
        ("bell", Icon::Bell, "Activity", "panel.activity"),
    ] {
        let r = Rect::from_center_size(pos2(x, full.center().y), vec2(24.0, 24.0));
        let tip = crate::i18n::tr(tip);
        let resp = ui.interact(r, egui::Id::unique(("top", id)), Sense::click());
        let name = crate::widgets::accessible_label(&resp, tip);
        let resp = resp.on_hover_text(tip);
        crate::widgets::announce(&resp, name);
        register(ui, format!("icon:{id}"), r);
        let warn = id == "cloud" && unsaved.is_some();
        let colour = if warn {
            t.caution
        } else if resp.hovered() {
            t.text
        } else {
            t.icon
        };
        paint(ui.painter(), r.shrink(4.0), icon, colour);
        if warn {
            let c = r.right_top() + vec2(-4.0, 5.0);
            ui.painter().circle_filled(c, 5.5, t.reject);
            ui.painter().text(c, Align2::CENTER_CENTER, "!", t.semibold(9.0), t.canvas);
            register(ui, "indicator:unsaved", r);
        }
        if id == "import" {
            // "+" opens the import menu: photos, a folder, and the cameras and cards plugged in
            egui::Popup::menu(&resp).show(|ui| import_menu(app, ui));
        } else if resp.clicked()
            && !cmd.is_empty()
            && let Err(e) = app.run(cmd, json!({}))
        {
            // a refusal says why (Import while a Lightroom catalog is coming in)
            app.toast_error(ui.ctx(), e);
        }
        x -= 34.0;
    }
    x
}

/// The "+" button's menu: Import Photos…, Import from Folder…, then one entry per camera or card
/// found (a camera over USB opens the Import module; a card the import review), or a disabled
/// "No Camera or Card Found".
fn import_menu(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    ui.set_min_width(220.0);
    for (wid, cmd, label) in [("importPhotos", "file.addPhotos", "Import Photos…"), ("importFolder", "file.addFolder", "Import from Folder…")] {
        let enabled = crate::menus::ui_enabled(app, cmd);
        let r = ui.add_enabled(enabled, egui::Button::new(crate::i18n::tr(label)));
        register(ui, format!("menu:{wid}"), r.rect);
        if r.clicked() {
            if let Err(e) = app.run(cmd, json!({})) {
                app.toast_error(ui.ctx(), e);
            }
            ui.close();
        }
    }
    ui.separator();
    let devices = lightcraft_engine::devices::devices();
    if devices.is_empty() {
        let r = ui.add_enabled(false, egui::Button::new(crate::i18n::tr("No Camera or Card Found")));
        register(ui, "menu:importDevice-none", r.rect);
        return;
    }
    for (i, d) in devices.iter().enumerate() {
        let r = ui.button(format!("{}…", d.name));
        register(ui, format!("menu:importDevice-{i}"), r.rect);
        if r.clicked() {
            let result = match &d.camera {
                Some(camera) => app.run("import.camera", json!({"camera": camera})),
                None => app.run("file.addFromDevice", json!({"path": d.path})),
            };
            if let Err(e) = result {
                app.toast_error(ui.ctx(), e);
            }
            ui.close();
        }
    }
}

/// The appearance icon at `x`, with its theme-favourites menu; returns the x of the next control.
fn appearance_button(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, full: Rect, x: f32) -> f32 {
    // Appearance Mode: a click steps Auto → Light → Dark → Auto; the icon shows the mode
    let (icon, mode) = appearance_icon(app.ui.settings.appearance_mode);
    let r = Rect::from_center_size(pos2(x, full.center().y), vec2(24.0, 24.0));
    let tip = format!("{}: {}", crate::i18n::tr("Appearance Mode"), crate::i18n::tr(mode));
    let resp = ui.interact(r, egui::Id::unique(("top", "appearance")), Sense::click()).on_hover_text(tip.clone());
    crate::widgets::announce(&resp, tip.clone());
    register(ui, "icon:appearance", r);
    paint(ui.painter(), r.shrink(4.0), icon, if resp.hovered() { t.text } else { t.icon });
    if resp.clicked() {
        let _ = app.run("view.appearance", json!({}));
    }
    crate::widgets::context_menu(&resp, |ui| {
        let settings = ui.button(crate::i18n::tr("Appearance Settings…"));
        register(ui, "menu:appearanceSettings", settings.rect);
        if settings.clicked() {
            let _ = app.run("app.settings", json!({"tab": "interface"}));
            ui.close();
        }
        let favorites = app.ui.settings.favorite_themes.clone();
        for dark in [false, true] {
            let group: Vec<_> =
                crate::theme::ThemeKind::ALL.iter().filter(|k| k.is_dark() == dark && favorites.iter().any(|id| id == k.id())).collect();
            if !group.is_empty() {
                ui.separator();
            }
            for kind in group {
                let r = ui.selectable_label(app.ui.settings.theme(app.system_theme(ui.ctx())) == *kind, crate::i18n::tr(kind.label()));
                register(ui, format!("menu:themeFavorite-{}", kind.id()), r.rect);
                if r.clicked() {
                    let _ = app.run("view.theme", json!({"theme": kind.id()}));
                    ui.close();
                }
            }
        }
    });
    x - 34.0
}

/// The appearance button's icon and the mode's name for its tooltip.
pub fn appearance_icon(mode: crate::state::AppearanceMode) -> (Icon, &'static str) {
    use crate::state::AppearanceMode;
    match mode {
        AppearanceMode::Auto => (Icon::Monitor, "Sync with System"),
        AppearanceMode::Light => (Icon::Sun, "Light Mode"),
        AppearanceMode::Dark => (Icon::Moon, "Dark Mode"),
    }
}

/// Tooltip for the warning cloud icon: `n` changes held in memory because saving failed with `e`.
fn unsaved_tip(n: usize, e: &str) -> String {
    let s = crate::i18n::plural_s(n);
    let it = if n == 1 { "it" } else { "them" };
    crate::i18n::tr_format!(
        "{n} change{} saved in memory but not written to disk: {e}\nLightspot retries automatically; quitting now would lose {}.",
        s,
        it,
        e = e,
        n = n
    )
}
