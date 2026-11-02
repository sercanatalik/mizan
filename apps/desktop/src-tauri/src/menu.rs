//! Native application menu. Clicks are forwarded to the frontend as events.

use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{App, AppHandle, Emitter, Runtime};

/// Sidebar sections in order; View › item n gets Cmd/Ctrl+n.
pub const SECTIONS: [(&str, &str); 9] = [
    ("overview", "Overview"),
    ("holdings", "Holdings"),
    ("activity", "Activity"),
    ("performance", "Performance"),
    ("income", "Income"),
    ("imports", "Imports"),
    ("accounts", "Accounts"),
    ("assets", "Assets"),
    ("settings", "Settings"),
];

pub const EVENT_IMPORT: &str = "menu:import";
pub const EVENT_NAVIGATE: &str = "menu:navigate";

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

    let import = MenuItemBuilder::with_id("import", "Import file…")
        .accelerator("CmdOrCtrl+O")
        .build(app)?;
    let file = SubmenuBuilder::new(app, "File")
        .item(&import)
        .separator()
        .close_window()
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

    let mut view = SubmenuBuilder::new(app, "View");
    for (i, (id, label)) in SECTIONS.iter().enumerate() {
        let item = MenuItemBuilder::with_id(format!("nav:{id}"), *label)
            .accelerator(format!("CmdOrCtrl+{}", i + 1))
            .build(app)?;
        view = view.item(&item);
    }
    let view = view.build()?;

    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .separator()
        .close_window()
        .build()?;

    let menu = MenuBuilder::new(app)
        .items(&[&app_menu, &file, &edit, &view, &window])
        .build()?;
    app.set_menu(menu)?;
    app.on_menu_event(|handle, event| handle_event(handle, event.id().as_ref()));
    Ok(())
}

fn handle_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let _ = match id {
        "import" => app.emit(EVENT_IMPORT, ()),
        _ => match id.strip_prefix("nav:") {
            Some(section) => app.emit(EVENT_NAVIGATE, section),
            None => Ok(()),
        },
    };
}
