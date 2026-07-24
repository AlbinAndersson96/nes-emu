/// Result of one frame's menu interaction: which File-menu item, if any,
/// was clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    None,
    LoadRom,
    SaveState,
    LoadState,
    PlayReplay,
}

/// Draws the File menu and returns which item (if any) was clicked this frame.
/// `rom_loaded` gates the save/load-state items, which are meaningless without
/// a running machine (a save state is tied to the loaded ROM).
pub fn draw(ui: &mut egui::Ui, rom_loaded: bool) -> MenuAction {
    let mut action = MenuAction::None;
    egui::Panel::top("menu_bar").show(ui, |ui| {
        egui::containers::menu::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Load ROM").clicked() {
                    action = MenuAction::LoadRom;
                    ui.close();
                }
                ui.separator();
                if ui
                    .add_enabled(rom_loaded, egui::Button::new("Save State..."))
                    .clicked()
                {
                    action = MenuAction::SaveState;
                    ui.close();
                }
                if ui
                    .add_enabled(rom_loaded, egui::Button::new("Load State..."))
                    .clicked()
                {
                    action = MenuAction::LoadState;
                    ui.close();
                }
                ui.separator();
                if ui.button("Play Replay...").clicked() {
                    action = MenuAction::PlayReplay;
                    ui.close();
                }
            });
        });
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_does_not_panic_and_reports_no_action_by_default() {
        let ctx = egui::Context::default();
        let raw_input = egui::RawInput::default();
        let mut action = MenuAction::None;
        let _ = ctx.run_ui(raw_input, |ui| {
            action = draw(ui, true);
        });
        assert_eq!(
            action,
            MenuAction::None,
            "no simulated click occurred, so draw() must report MenuAction::None"
        );
    }
}
