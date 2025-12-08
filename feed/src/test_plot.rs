use egui_plot::{Plot, Scale};

fn main() {
    let _ = Plot::new("test").x_scale(Scale::Log10);
}
