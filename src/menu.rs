use muda::{Menu, MenuId, MenuItem, Submenu};

pub struct AppMenuIds {
    pub load_rom: MenuId,
}

fn build_menu() -> Menu {
    let menu = Menu::new();
    let file_menu = Submenu::new("File", true);
    let load_rom = MenuItem::new("Load ROM", true, None);
    file_menu
        .append(&load_rom)
        .expect("appending Load ROM to the File submenu cannot fail");
    menu.append(&file_menu)
        .expect("appending the File submenu to the menu bar cannot fail");
    menu
}

/// Builds the menu bar and attaches it to `window`'s native chrome
/// (GTK menu bar on Linux, HWND system menu on Windows). Returns the
/// `MenuId`s the caller needs to match `muda::MenuEvent`s against.
#[cfg(target_os = "linux")]
pub fn build_and_attach(window: &tao::window::Window) -> AppMenuIds {
    use tao::platform::unix::WindowExtUnix;

    let menu = build_menu();
    let load_rom = find_load_rom_id(&menu);
    menu.init_for_gtk_window(window.gtk_window(), window.default_vbox())
        .expect("attaching the menu bar to the GTK window cannot fail");
    AppMenuIds { load_rom }
}

#[cfg(target_os = "windows")]
pub fn build_and_attach(window: &tao::window::Window) -> AppMenuIds {
    use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

    let menu = build_menu();
    let load_rom = find_load_rom_id(&menu);
    // SAFETY: `window` is a live, open window for the lifetime of the
    // program; `hwnd` is only used here, synchronously, to attach the menu.
    unsafe {
        match window.raw_window_handle() {
            RawWindowHandle::Win32(handle) => {
                menu.init_for_hwnd(handle.hwnd as isize)
                    .expect("attaching the menu bar to the HWND cannot fail");
            }
            _ => unreachable!("windows target must produce a Win32 window handle"),
        }
    }
    AppMenuIds { load_rom }
}

fn find_load_rom_id(menu: &Menu) -> MenuId {
    let items = menu.items();
    let file_submenu = items[0]
        .as_submenu()
        .expect("first item is always the File submenu built by build_menu");
    let file_items = file_submenu.items();
    let load_rom = file_items[0]
        .as_menuitem()
        .expect("first item is always the Load ROM item built by build_menu");
    load_rom.id().clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_menu_has_one_load_rom_item() {
        let menu = build_menu();
        let items = menu.items();
        assert_eq!(items.len(), 1, "expected exactly one top-level submenu (File)");

        let file_submenu = items[0]
            .as_submenu()
            .expect("top-level item must be the File submenu");
        assert_eq!(file_submenu.text(), "File");

        let file_items = file_submenu.items();
        assert_eq!(file_items.len(), 1, "expected exactly one item in File");

        let load_rom = file_items[0]
            .as_menuitem()
            .expect("File's item must be a plain MenuItem");
        assert_eq!(load_rom.text(), "Load ROM");
    }
}
