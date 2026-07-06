/// Draws the File menu with a single "Load ROM" item. Returns `true` if it
/// was clicked this frame.
pub fn draw(ui: &mut egui::Ui) -> bool {
    let mut load_rom_clicked = false;
    egui::Panel::top("menu_bar").show(ui, |ui| {
        egui::containers::menu::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Load ROM").clicked() {
                    load_rom_clicked = true;
                    ui.close();
                }
            });
        });
    });
    load_rom_clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_does_not_panic_and_reports_no_click_by_default() {
        let ctx = egui::Context::default();
        let raw_input = egui::RawInput::default();
        let mut clicked = false;
        let _ = ctx.run_ui(raw_input, |ui| {
            clicked = draw(ui);
        });
        assert!(
            !clicked,
            "no simulated click occurred, so draw() must report false"
        );
    }
}
