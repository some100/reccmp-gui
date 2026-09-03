use crate::app::App;

mod app;
mod disassemble;
mod reccmp;
mod roadmap;
mod stackcmp;
mod worker;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "reccmp-gui",
        native_options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
