#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod chatbox;
mod egui_overlay;
mod openvr;
mod vr_input;
use eframe::egui;
use std::*;
//use wana_kana::ConvertJapanese;

type Vector2 = nalgebra::Vector2<f32>;

struct Model {
    interval: time::Duration,
    current_strokes: [Vec<Vector2>; 2],
    text: Vec<char>,
    cursor: usize,
    indicator: char,
}

struct Ui {}

struct App {
    time: time::Instant,
    model: Model,
    system: openvr::System,
    vr_input: vr_input::VrInput,
    recognizer: graffiti_3d::GraffitiRecognizer,
    chatbox: Option<chatbox::ChatBox>,
    overlay: egui_overlay::EguiOverlay,
    ui: Ui,
}

fn v2_invert_y(v: Vector2) -> Vector2 {
    Vector2::new(v[0], -v[1])
}

fn sleep_high_res(d: time::Duration) {
    #[cfg(target_os = "windows")]
    {
        extern "C" {
            fn sleep_100ns(_: i64) -> bool;
        }
        let t = (d.as_nanos() / 100).try_into().unwrap();
        let r = unsafe { sleep_100ns(t) };
        assert!(r);
    }
    #[cfg(not(target_os = "windows"))]
    thread::sleep(d);
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let overlay = egui_overlay::EguiOverlay::new(
            cc.gl.as_ref().unwrap().clone(),
            &[512, 512],
            b"GraffitiVR\0",
        );

        App {
            time: time::Instant::now(),
            model: Model {
                interval: time::Duration::from_secs(1) / 90,
                current_strokes: [Vec::new(), Vec::new()],
                text: Vec::new(),
                cursor: 0,
                indicator: ' ',
            },
            system: openvr::System::new(),
            vr_input: vr_input::VrInput::new(),
            recognizer: graffiti_3d::GraffitiRecognizer::new(0.02),
            chatbox: chatbox::ChatBox::new().ok(),
            ui: Ui::new(&cc.egui_ctx, &overlay.context),
            overlay: overlay,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        sleep_high_res(self.model.interval.saturating_sub(self.time.elapsed()));
        self.time = time::Instant::now();

        self.vr_input.update(&self.system);
        self.model.current_strokes = self.vr_input.current_strokes();

        if let Some(stroke) = self.vr_input.pop_stroke() {
            match self.recognizer.recognize(&stroke) {
                Some('\x08') => {
                    if self.model.cursor > 0 {
                        self.model.cursor -= 1;
                        self.model.text.remove(self.model.cursor);
                    }
                }
                Some('←') => {
                    self.model.cursor = cmp::max(self.model.cursor, 1) - 1;
                }
                Some('→') => {
                    self.model.cursor = cmp::min(self.model.cursor + 1, self.model.text.len());
                }
                Some('\n') => {
                    self.model.text.clear();
                    self.model.cursor = 0;
                }
                Some(c) => {
                    self.model.text.insert(self.model.cursor, c);
                    self.model.cursor += 1;
                }
                None => (),
            }
        }
        self.model.indicator = match self.recognizer.modifier() {
            graffiti_3d::GraffitiModifier::Symbol => '.',
            graffiti_3d::GraffitiModifier::Caps => '^',
            graffiti_3d::GraffitiModifier::None => match self.recognizer.mode() {
                graffiti_3d::GraffitiMode::Number => '#',
                _ => ' ',
            },
        };

        if let Some(ref mut chatbox) = self.chatbox {
            let text: String = self.model.text.iter().collect();
            chatbox.input(text /*.to_hiragana()*/);
            chatbox.typing(self.model.current_strokes.iter().any(|s| s.len() > 0));
            chatbox.update();
        }

        self.overlay.run(|ctx| self.ui.overlay(ctx, &self.model));
        self.ui.main(ctx, &self.model);

        ctx.request_repaint();
    }
}

impl Ui {
    fn new(ctx_main: &egui::Context, ctx_overlay: &egui::Context) -> Self {
        Self::add_font_ja(ctx_main);
        Self::add_font_ja(ctx_overlay);

        Ui {}
    }

    fn main(&self, ctx: &egui::Context, model: &Model) {
        egui::CentralPanel::default().show(ctx, |ui| {
            self.text(ui, model);
            self.plot(ui, model);
        });
    }

    fn overlay(&self, ctx: &egui::Context, model: &Model) {
        let frame = egui::Frame::none();
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            self.text(ui, model);
            self.plot(ui, model);
        });
    }

    fn text(&self, ui: &mut egui::Ui, model: &Model) {
        let lhs = model.text[..model.cursor]
            .iter()
            .collect::<String>()
            /*.to_hiragana()*/;
        let rhs = model.text[model.cursor..]
            .iter()
            .collect::<String>()
            /*.to_hiragana()*/;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            ui.label(
                egui::RichText::new(lhs)
                    .size(24.0)
                    .color(egui::Color32::from_rgb(255, 255, 255)),
            );
            ui.label(
                egui::RichText::new(model.indicator)
                    .size(24.0)
                    .color(egui::Color32::from_rgb(0, 0, 0))
                    .background_color(egui::Color32::from_rgb(128, 192, 255)),
            );
            ui.label(
                egui::RichText::new(rhs)
                    .size(24.0)
                    .color(egui::Color32::from_rgb(255, 255, 255)),
            );
        });
    }

    fn plot(&self, ui: &mut egui::Ui, model: &Model) {
        let (response, painter) =
            ui.allocate_painter(ui.available_size_before_wrap(), egui::Sense::drag());
        let r_min = Vector2::new(response.rect.min.x, response.rect.min.y);
        let r_max = Vector2::new(response.rect.max.x, response.rect.max.y);

        for stroke in model.current_strokes.iter() {
            if stroke.len() < 2 {
                continue;
            }
            let mut s_min = Vector2::repeat(f32::INFINITY);
            let mut s_max = Vector2::repeat(-f32::INFINITY);
            for v in stroke.iter() {
                let v = v2_invert_y(*v);
                s_min = s_min.inf(&v);
                s_max = s_max.sup(&v);
            }
            let scale = (r_max - r_min).component_div(&(s_max - s_min)).min();
            let offset = 0.5 * ((r_max + r_min) - scale * (s_max + s_min));

            let egui_stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 255, 255));
            for i in 0..stroke.len() - 1 {
                let v0 = scale * v2_invert_y(stroke[i + 0]) + offset;
                let v1 = scale * v2_invert_y(stroke[i + 1]) + offset;
                painter.line_segment(
                    [egui::Pos2::new(v0[0], v0[1]), egui::Pos2::new(v1[0], v1[1])],
                    egui_stroke,
                );
            }
        }
    }

    fn add_font_ja(ctx: &egui::Context) {
        let mut font = egui::FontDefinitions::default();
        font.font_data.insert(
            "mplus".to_owned(),
            egui::FontData::from_static(include_bytes!("../assets/mplus-1c-regular-sub.ttf"))
                .tweak(egui::FontTweak {
                    scale: 1.0,
                    y_offset_factor: 0.0,
                    y_offset: -12.0,
                }),
        );
        font.families
            .get_mut(&egui::FontFamily::Monospace)
            .unwrap()
            .push("mplus".to_owned());
        font.families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .push("mplus".to_owned());
        ctx.set_fonts(font);
    }
}

fn main() -> eframe::Result<()> {
    assert!(openvr::init());

    let mut opt = eframe::NativeOptions::default();
    opt.vsync = false;
    let result = eframe::run_native(
        "GraffitiVR",
        opt,
        Box::new(move |cc| Box::new(App::new(cc))),
    );

    openvr::shutdown();
    result
}
