//! The top bar's module picker (Library, Develop, Slideshow, Print) and identity plate, as
//! Lightroom Classic arranges them, in place of the search field and the community link.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::state::{RightPanel, ViewMode};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(10);

fn app() -> Headless {
    let services = Services { png: None, ..Default::default() };
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    app.native_menu = true;
    let mut h = Headless::new(app, [1200.0, 800.0], 1.0);
    h.settle(Duration::from_secs(120));
    h
}

fn widget(h: &Headless, id: &str) -> Option<egui::Rect> {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r)
}

// Scenario: the bar carries the four modules and the identity plate, no search field or Discord link
#[test]
fn the_top_bar_has_the_module_picker_and_the_identity_plate() {
    let h = app();
    for id in ["module:library", "module:develop", "module:slideshow", "module:print", "brand:identity", "icon:filter", "icon:cloud"] {
        assert!(widget(&h, id).is_some(), "{id} is on the bar");
    }
    for id in ["field:search", "icon:discord"] {
        assert!(widget(&h, id).is_none(), "{id} left the bar");
    }
    let top = crate::theme::Tokens::default().top_bar_h;
    let plate = widget(&h, "brand:identity").unwrap();
    let modules = widget(&h, "modules").unwrap();
    assert!(plate.bottom() <= top && modules.bottom() <= top, "both sit in the top bar");
    assert!(plate.right() < modules.left(), "the plate is left of the modules, as Lightroom's identity plate");
    assert!(modules.right() < widget(&h, "icon:filter").unwrap().left(), "the modules end before the icons");
}

// Scenario: Library shows the grid, Develop the photo with its sliders; the lit module follows
#[test]
fn clicking_a_module_switches_to_it() {
    let mut h = app();
    let r = h.request("ui.clickWidget", json!({"id": "module:library"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(Duration::from_secs(10));
    assert_eq!(h.app.ui.view, ViewMode::PhotoGrid);
    let r = h.request("ui.clickWidget", json!({"id": "module:develop"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(Duration::from_secs(10));
    assert_eq!(h.app.ui.view, ViewMode::Detail);
    assert!(h.app.ui.right.is_edit_tool(), "Develop brings the sliders: {:?}", h.app.ui.right);
    assert_ne!(h.app.ui.right, RightPanel::None);
}

// Scenario: Cmd+F lands in the filter bar's text field, where the search lives now
#[test]
fn focus_search_opens_the_filter_bar_and_lands_in_its_field() {
    let mut h = app();
    assert!(!h.app.ui.filter_bar);
    let r = h.request("engine.execute", json!({"command": "view.focusSearch"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(Duration::from_secs(10));
    assert!(h.app.ui.filter_bar, "the filter bar opens");
    assert_eq!(h.app.ui.view, ViewMode::PhotoGrid, "in the grid, where the bar shows");
    assert!(widget(&h, "field:search").is_some(), "the field is on the bar");
    let r = h.request("ui.text", json!({"text": "dune"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(Duration::from_secs(10));
    assert_eq!(h.app.session.filter.text, "dune", "typing filters the library");
}

// Scenario: the tool strip's lit Presets button closes a Presets column folded down to its icon,
// instead of unfolding it (it had stayed, minimized, beside the Tools panel)
#[test]
fn the_presets_button_closes_a_presets_column_folded_to_its_icon() {
    use crate::docking::Panel;
    let mut h = app();
    h.app.run("panel.presets", json!({})).unwrap();
    h.step();
    h.app.run("ui.dock", json!({"operation": "activate", "panel": "presets"})).unwrap();
    h.step();
    assert!(h.app.ui.presets && crate::docking::exposed(&h.app, Panel::Presets));
    // fold the column that holds Presets to icons, as its collapse button does
    let workspace = h.app.ui.docking.as_mut().expect("a docking workspace");
    let mut folded = false;
    let mut nodes = workspace.layout.root.iter_mut().collect::<Vec<_>>();
    while let Some(node) = nodes.pop() {
        match node {
            craft_ui::docking::Node::Split { first, second, .. } => nodes.extend([first.as_mut(), second.as_mut()]),
            craft_ui::docking::Node::Dock { columns, .. } => {
                for column in columns.iter_mut().filter(|c| c.groups.iter().any(|g| g.panels.contains(&Panel::Presets))) {
                    column.iconic = true;
                    folded = true;
                }
            }
            _ => {}
        }
    }
    assert!(folded, "Presets sits in a dock column");
    h.step();
    assert!(!crate::docking::exposed(&h.app, Panel::Presets), "folded to its icon, Presets is out of sight");
    h.app.run("panel.presets", json!({})).unwrap();
    h.step();
    assert!(!h.app.ui.presets, "the button closes it");
    assert!(!h.app.ui.docking.as_ref().unwrap().layout.contains(&Panel::Presets), "and it leaves the layout");
}
