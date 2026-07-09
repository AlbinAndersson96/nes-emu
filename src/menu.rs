/// Result of one frame's menu interaction: which File-menu item, if any,
/// was clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    None,
    LoadRom,
    PlayReplay,
}

/// Draws the File menu with "Load ROM" and "Play Replay..." items. Returns
/// which one (if any) was clicked this frame.
pub fn draw(ui: &mut egui::Ui) -> MenuAction {
    let mut action = MenuAction::None;
    egui::Panel::top("menu_bar").show(ui, |ui| {
        egui::containers::menu::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Load ROM").clicked() {
                    action = MenuAction::LoadRom;
                    ui.close();
                }
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
            action = draw(ui);
        });
        assert_eq!(
            action,
            MenuAction::None,
            "no simulated click occurred, so draw() must report MenuAction::None"
        );
    }
}
