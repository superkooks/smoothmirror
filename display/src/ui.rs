use std::sync::{Arc, Mutex};

use egui_glium::EguiGlium;
use glium::{glutin::surface::WindowSurface, winit::window::Window, Display};

pub struct Ui {
    pub open: bool,
    pub egui_glium: EguiGlium,

    pub volume: Arc<Mutex<f32>>,
    pub quit: bool,
}

impl Ui {
    pub fn redraw<T: glium::Surface>(
        &mut self,
        window: &Window,
        display: &Display<WindowSurface>,
        target: &mut T,
    ) {
        self.egui_glium.run(window, |egui_ctx| {
            if !self.open {
                return;
            }

            egui_ctx.set_zoom_factor(1.3);

            egui::SidePanel::left("left")
                .resizable(false)
                .show(egui_ctx, |ui| {
                    ui.heading("Settings");
                    ui.label("   F7 to close");

                    ui.add_space(15.);

                    ui.label("Volume");
                    ui.add(egui::Slider::new(
                        &mut *self.volume.lock().unwrap(),
                        0.0..=100.0,
                    ));

                    ui.add_space(15.);

                    if ui.button("Quit").clicked() {
                        self.quit = true;
                    }
                });
        });

        self.egui_glium.paint(display, target);
    }
}
