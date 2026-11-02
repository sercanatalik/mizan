//! Native application menu. Clicks are forwarded to the frontend as events.

use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{App, AppHandle, Emitter, Runtime};

/// Sidebar sections in order, mirroring `src/app/sections.ts`; a test checks
/// this list against `sections.json`, written by `npm run gen:sections`.
/// Go › item n gets Cmd/Ctrl+n for the first nine, and Settings gets the
/// platform's usual Cmd/Ctrl+comma.
pub const SECTIONS: [(&str, &str); 14] = [
    ("overview", "Overview"),
    ("inbox", "Inbox"),
    ("holdings", "Holdings"),
    ("accounts", "Accounts"),
    ("performance", "Performance"),
    ("income", "Income"),
    ("activity", "Activity"),
    ("imports", "Imports"),
    ("adapters", "Adapters"),
    ("tax", "Tax"),
    ("exports", "Exports"),
    ("assets", "Assets"),
    ("market-data", "Market data"),
    ("settings", "Settings"),
];

/// The Cmd/Ctrl shortcut key of section `i` in [`SECTIONS`].
pub fn section_key(i: usize) -> Option<String> {
    if SECTIONS[i].0 == "settings" {
        Some(",".to_string())
    } else {
        (i < 9).then(|| (i + 1).to_string())
    }
}

pub const EVENT_IMPORT: &str = "menu:import";
pub const EVENT_NAVIGATE: &str = "menu:navigate";

/// A custom menu item: id (also its event), label and accelerator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub label: String,
    pub accelerator: Option<String>,
}

fn item(id: &str, label: &str, accel: Option<&str>) -> Item {
    Item {
        id: id.into(),
        label: label.into(),
        accelerator: accel.map(Into::into),
    }
}

/// The app's own menu items, by menu, separators as `None`. `dev` builds keep
/// Cmd/Ctrl+R for reloading the webview, so Nominal | Real has no shortcut there.
pub fn menu_spec(dev: bool) -> Vec<(&'static str, Vec<Option<Item>>)> {
    let go: Vec<Option<Item>> = SECTIONS
        .iter()
        .enumerate()
        .map(|(i, (id, label))| {
            Some(Item {
                id: format!("nav:{id}"),
                label: (*label).into(),
                accelerator: section_key(i).map(|k| format!("CmdOrCtrl+{k}")),
            })
        })
        .chain([None, Some(item("find", "Find…", Some("CmdOrCtrl+K")))])
        .collect();
    vec![
        (
            "File",
            vec![
                Some(item("import", "Import file…", Some("CmdOrCtrl+O"))),
                Some(item("add-activity", "Add activity", Some("CmdOrCtrl+N"))),
                None,
                Some(item("exports", "Exports…", Some("CmdOrCtrl+E"))),
                Some(item("backup", "Back up database…", None)),
            ],
        ),
        ("Go", go),
        (
            "View",
            vec![
                Some(item(
                    "toggle-real",
                    "Nominal | Real",
                    (!dev).then_some("CmdOrCtrl+R"),
                )),
                Some(item("inspector", "Inspector", Some("CmdOrCtrl+I"))),
            ],
        ),
        (
            "Data",
            vec![
                Some(item("refresh", "Refresh prices", Some("CmdOrCtrl+Shift+R"))),
                Some(item("undo-import", "Undo last import", None)),
            ],
        ),
    ]
}

/// The event a menu item sends to the frontend, with its payload.
pub fn event_for(id: &str) -> Option<(&'static str, Option<String>)> {
    Some(match id {
        "import" => (EVENT_IMPORT, None),
        "add-activity" => ("menu:add-activity", None),
        "exports" => ("menu:exports", None),
        "backup" => ("menu:backup", None),
        "find" => ("menu:find", None),
        "toggle-real" => ("menu:toggle-real", None),
        "inspector" => ("menu:inspector", None),
        "refresh" => ("menu:refresh", None),
        "undo-import" => ("menu:undo-import", None),
        _ => {
            return id
                .strip_prefix("nav:")
                .map(|s| (EVENT_NAVIGATE, Some(s.to_string())))
        }
    })
}

pub fn build<R: Runtime>(app: &App<R>) -> tauri::Result<()> {
    let app_menu = SubmenuBuilder::new(app, "Mizan")
        .about(None)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let mut menus = vec![app_menu];
    for (name, items) in menu_spec(cfg!(debug_assertions)) {
        let mut sub = SubmenuBuilder::new(app, name);
        for entry in items {
            sub = match entry {
                None => sub.separator(),
                Some(it) => {
                    let mut b = MenuItemBuilder::with_id(it.id, it.label);
                    if let Some(a) = it.accelerator {
                        b = b.accelerator(a);
                    }
                    sub.item(&b.build(app)?)
                }
            };
        }
        if name == "File" {
            sub = sub.separator().close_window();
        }
        menus.push(sub.build()?);
        if name == "File" {
            menus.push(edit.clone());
        }
    }
    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .separator()
        .close_window()
        .build()?;
    menus.push(window);

    let refs: Vec<&dyn tauri::menu::IsMenuItem<R>> = menus
        .iter()
        .map(|m| m as &dyn tauri::menu::IsMenuItem<R>)
        .collect();
    let menu = MenuBuilder::new(app).items(&refs).build()?;
    app.set_menu(menu)?;
    app.on_menu_event(|handle, event| handle_event(handle, event.id().as_ref()));
    Ok(())
}

fn handle_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    if let Some((event, payload)) = event_for(id) {
        let _ = match payload {
            Some(p) => app.emit(event, p),
            None => app.emit(event, ()),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Entry {
        id: String,
        label: String,
        shortcut: Option<usize>,
    }

    /// The sidebar list, as written by `npm run gen:sections`.
    fn sidebar() -> Vec<Entry> {
        serde_json::from_str(include_str!("../sections.json")).expect("sections.json parses")
    }

    #[test]
    fn menu_sections_match_the_sidebar() {
        let sidebar = sidebar();
        let menu: Vec<(&str, &str)> = SECTIONS.to_vec();
        let side: Vec<(&str, &str)> = sidebar
            .iter()
            .map(|e| (e.id.as_str(), e.label.as_str()))
            .collect();
        assert_eq!(
            menu, side,
            "menu.rs SECTIONS differ from src/app/sections.ts; update one and run `npm run gen:sections`"
        );
    }

    fn accel(dev: bool, id: &str) -> Option<String> {
        menu_spec(dev)
            .into_iter()
            .flat_map(|(_, items)| items)
            .flatten()
            .find(|i| i.id == id)
            .unwrap_or_else(|| panic!("{id} in the menu"))
            .accelerator
    }

    #[test]
    fn menus_and_shortcuts() {
        let names: Vec<&str> = menu_spec(false).iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["File", "Go", "View", "Data"]);
        assert_eq!(accel(false, "import").as_deref(), Some("CmdOrCtrl+O"));
        assert_eq!(accel(false, "add-activity").as_deref(), Some("CmdOrCtrl+N"));
        assert_eq!(accel(false, "exports").as_deref(), Some("CmdOrCtrl+E"));
        assert_eq!(accel(false, "backup"), None);
        assert_eq!(accel(false, "nav:inbox").as_deref(), Some("CmdOrCtrl+2"));
        assert_eq!(accel(false, "nav:settings").as_deref(), Some("CmdOrCtrl+,"));
        assert_eq!(accel(false, "find").as_deref(), Some("CmdOrCtrl+K"));
        assert_eq!(accel(false, "toggle-real").as_deref(), Some("CmdOrCtrl+R"));
        assert_eq!(
            accel(true, "toggle-real"),
            None,
            "dev builds keep reload on Cmd+R"
        );
        assert_eq!(accel(false, "inspector").as_deref(), Some("CmdOrCtrl+I"));
        assert_eq!(
            accel(false, "refresh").as_deref(),
            Some("CmdOrCtrl+Shift+R")
        );
        assert_eq!(accel(false, "undo-import"), None);
    }

    #[test]
    fn no_shortcut_is_used_twice() {
        let mut seen = std::collections::HashSet::new();
        for a in menu_spec(false)
            .into_iter()
            .flat_map(|(_, i)| i)
            .flatten()
            .filter_map(|i| i.accelerator)
        {
            assert!(seen.insert(a.clone()), "{a} is used twice");
        }
    }

    #[test]
    fn every_item_sends_an_event() {
        for it in menu_spec(false).into_iter().flat_map(|(_, i)| i).flatten() {
            assert!(event_for(&it.id).is_some(), "{} sends nothing", it.id);
        }
        assert_eq!(
            event_for("nav:inbox"),
            Some((EVENT_NAVIGATE, Some("inbox".to_string())))
        );
        assert_eq!(event_for("find"), Some(("menu:find", None)));
    }

    #[test]
    fn menu_shortcuts_match_the_sidebar() {
        for (i, e) in sidebar().iter().enumerate() {
            let key = section_key(i);
            match e.shortcut {
                Some(n) => assert_eq!(key, Some(n.to_string()), "{}", e.id),
                None => assert!(
                    key.is_none() || e.id == "settings",
                    "{} has no digit shortcut",
                    e.id
                ),
            }
        }
    }
}
