//! Ohmforge desktop application.
//!
//! Without the `gui` feature this prints a notice; build with
//! `cargo run -p of-app --features gui` for the egui window.

#[cfg(feature = "gui")]
fn main() -> eframe::Result<()> {
    let mut viewport = eframe::egui::ViewportBuilder::default();
    // Window and taskbar icon (assets/icon-192.png); skipped if it fails to decode.
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon-192.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native("Ohmforge", options, Box::new(|_cc| Ok(Box::new(App))))
}

#[cfg(feature = "gui")]
struct App;

#[cfg(feature = "gui")]
impl eframe::App for App {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        eframe::egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Ohmforge");
            ui.label("ESP32 circuit simulator: pre-alpha scaffold.");
        });
    }
}

#[cfg(not(feature = "gui"))]
fn main() {
    println!("Ohmforge desktop app scaffold. Rebuild with `--features gui` for the window.");
}
