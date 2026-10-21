#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
#![expect(rustdoc::missing_crate_level_docs)] // it's an example

//! MEMBRANE: compare `RectShape` with `BorderRectShape`.
//!
//! The first section paints four equal sides with both shapes. `BorderRectShape` paints a
//! `RectShape` for four equal sides, so this demo calls `tessellate_border_rect_sides` to paint
//! the mesh for different sides instead. The two columns must look identical.
//!
//! The second section paints sides that `RectShape` cannot paint.

use eframe::egui::{
    self, Color32, MarginF32, Rect, RichText, Sense, Shape, Ui, Vec2,
    epaint::{BorderRectShape, Mesh, RectShape, SideColors, Stroke, StrokeKind, Tessellator},
    vec2,
};

fn main() -> eframe::Result {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 900.0]),
        ..Default::default()
    };

    let mut demo = Demo::default();
    eframe::run_ui_native("BorderRectShape", options, move |ui, _frame| {
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| demo.ui(ui));
        });
    })
}

struct Demo {
    fill: bool,
    translucent: bool,
    blink: bool,
    zoom: f32,
}

impl Default for Demo {
    fn default() -> Self {
        Self {
            fill: true,
            translucent: false,
            blink: false,
            zoom: 1.0,
        }
    }
}

const RADII: [u8; 7] = [0, 1, 3, 6, 10, 14, 18];
const WIDTHS: [f32; 7] = [0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 8.0];
const CELL: Vec2 = vec2(52.0, 36.0);

impl Demo {
    fn ui(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.fill, "Fill");
            ui.checkbox(&mut self.translucent, "Translucent stroke");
            ui.checkbox(&mut self.blink, "Blink")
                .on_hover_text("Paint the two shapes in the same place, one after the other");
            ui.add(egui::Slider::new(&mut self.zoom, 0.5..=4.0).text("Zoom"));
        });
        if ui.ctx().zoom_factor() != self.zoom {
            ui.ctx().set_zoom_factor(self.zoom);
        }
        ui.label(format!("Pixels per point: {}", ui.ctx().pixels_per_point()));

        ui.add_space(12.0);
        ui.heading("Four equal sides");
        ui.label(
            "In each cell, the left rectangle is a RectShape with StrokeKind::Inside. The right \
             rectangle is the mesh of BorderRectShape for different sides. The two must look \
             identical.",
        );
        self.equal_sides(ui);

        ui.add_space(12.0);
        ui.heading("Different sides");
        ui.label("Only BorderRectShape can paint these.");
        self.different_sides(ui);
    }

    fn fill(&self, ui: &Ui) -> Color32 {
        if self.fill {
            if ui.visuals().dark_mode {
                Color32::from_gray(60)
            } else {
                Color32::from_gray(215)
            }
        } else {
            Color32::TRANSPARENT
        }
    }

    fn stroke_color(&self) -> Color32 {
        if self.translucent {
            Color32::from_white_alpha(70)
        } else {
            Color32::from_rgb(230, 120, 30)
        }
    }

    fn equal_sides(&self, ui: &mut Ui) {
        let fill = self.fill(ui);
        let color = self.stroke_color();
        // Show the RectShape for half a second, then the mesh for half a second.
        let show_mesh = if self.blink {
            ui.ctx().request_repaint();
            ui.input(|i| i.time).fract() >= 0.5
        } else {
            false
        };

        egui::Grid::new("equal_sides")
            .spacing(vec2(16.0, 12.0))
            .show(ui, |ui| {
                ui.label("width \\ radius");
                for radius in RADII {
                    ui.label(radius.to_string());
                }
                ui.end_row();

                for width in WIDTHS {
                    ui.label(width.to_string());
                    for radius in RADII {
                        let rect_shape = RectShape::new(
                            Rect::ZERO,
                            radius,
                            fill,
                            Stroke::new(width, color),
                            StrokeKind::Inside,
                        );
                        let border_shape = BorderRectShape::new(
                            Rect::ZERO,
                            radius,
                            fill,
                            MarginF32::same(width),
                            color,
                        );
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let rect = allocate(ui, CELL);
                            let border_shape = BorderRectShape {
                                rect,
                                ..border_shape
                            };
                            if !self.blink {
                                ui.painter().add(RectShape { rect, ..rect_shape });
                                let rect = allocate(ui, CELL);
                                paint_sides_mesh(
                                    ui,
                                    &BorderRectShape {
                                        rect,
                                        ..border_shape
                                    },
                                );
                            } else if show_mesh {
                                paint_sides_mesh(ui, &border_shape);
                            } else {
                                ui.painter().add(RectShape { rect, ..rect_shape });
                            }
                        });
                    }
                    ui.end_row();
                }
            });
    }

    fn different_sides(&self, ui: &mut Ui) {
        let fill = self.fill(ui);
        let color = self.stroke_color();
        let red = Color32::from_rgb(230, 70, 70);
        let green = Color32::from_rgb(80, 200, 100);
        let blue = Color32::from_rgb(80, 140, 240);
        let yellow = Color32::from_rgb(240, 200, 60);
        let four_colors = SideColors {
            left: red,
            right: green,
            top: blue,
            bottom: yellow,
        };
        let sides = |left: f32, right: f32, top: f32, bottom: f32| MarginF32 {
            left,
            right,
            top,
            bottom,
        };

        let cases: Vec<(&str, u8, MarginF32, SideColors)> = vec![
            ("bottom 1", 0, sides(0.0, 0.0, 0.0, 1.0), color.into()),
            (
                "bottom 1, radius 8",
                8,
                sides(0.0, 0.0, 0.0, 1.0),
                color.into(),
            ),
            (
                "bottom 2, radius 18",
                18,
                sides(0.0, 0.0, 0.0, 2.0),
                color.into(),
            ),
            (
                "bottom 0.5, radius 6",
                6,
                sides(0.0, 0.0, 0.0, 0.5),
                color.into(),
            ),
            (
                "top and bottom 1",
                0,
                sides(0.0, 0.0, 1.0, 1.0),
                color.into(),
            ),
            (
                "top and bottom 1, radius 8",
                8,
                sides(0.0, 0.0, 1.0, 1.0),
                color.into(),
            ),
            ("left 4", 0, sides(4.0, 0.0, 0.0, 0.0), color.into()),
            (
                "left 4, radius 8",
                8,
                sides(4.0, 0.0, 0.0, 0.0),
                color.into(),
            ),
            (
                "left 4 and 1 on the others, radius 8",
                8,
                sides(4.0, 1.0, 1.0, 1.0),
                color.into(),
            ),
            ("1, 2, 3, 4", 0, sides(1.0, 2.0, 3.0, 4.0), color.into()),
            (
                "1, 2, 3, 4, radius 12",
                12,
                sides(1.0, 2.0, 3.0, 4.0),
                color.into(),
            ),
            (
                "radius 3, smaller than the width",
                3,
                sides(8.0, 2.0, 2.0, 8.0),
                color.into(),
            ),
            ("four colors, width 4", 0, MarginF32::same(4.0), four_colors),
            (
                "four colors, width 4, radius 12",
                12,
                MarginF32::same(4.0),
                four_colors,
            ),
            (
                "four colors, width 1, radius 6",
                6,
                MarginF32::same(1.0),
                four_colors,
            ),
            (
                "four colors, 2, 6, 2, 6, radius 14",
                14,
                sides(2.0, 6.0, 2.0, 6.0),
                four_colors,
            ),
            (
                "translucent colors, width 6, radius 10",
                10,
                MarginF32::same(6.0),
                SideColors {
                    left: red.gamma_multiply(0.5),
                    right: green.gamma_multiply(0.5),
                    top: blue.gamma_multiply(0.5),
                    bottom: yellow.gamma_multiply(0.5),
                },
            ),
            (
                "transparent top, width 3, radius 10",
                10,
                MarginF32::same(3.0),
                SideColors {
                    top: Color32::TRANSPARENT,
                    ..four_colors
                },
            ),
            (
                "bottom 4 on a rect 3 high",
                2,
                sides(0.0, 0.0, 0.0, 4.0),
                color.into(),
            ),
        ];

        egui::Grid::new("different_sides")
            .spacing(vec2(24.0, 16.0))
            .show(ui, |ui| {
                for (index, (name, radius, widths, colors)) in cases.into_iter().enumerate() {
                    ui.vertical(|ui| {
                        let size = if name.ends_with("3 high") {
                            vec2(150.0, 3.0)
                        } else {
                            vec2(150.0, 60.0)
                        };
                        let rect = allocate(ui, size);
                        ui.painter()
                            .add(BorderRectShape::new(rect, radius, fill, widths, colors));
                        ui.label(RichText::new(name).small());
                    });
                    if index % 5 == 4 {
                        ui.end_row();
                    }
                }
            });
    }
}

fn allocate(ui: &mut Ui, size: Vec2) -> Rect {
    ui.allocate_exact_size(size, Sense::hover()).0
}

/// Paint the mesh of `BorderRectShape` for different sides, also when the sides are equal.
///
/// The demo tessellates the shape at the pixels per point of the screen. The mesh is not correct
/// in a layer with a scale, but this demo has no such layer.
fn paint_sides_mesh(ui: &Ui, shape: &BorderRectShape) {
    let ctx = ui.ctx();
    let options = ctx.tessellation_options(|options| *options);
    let mut tessellator = Tessellator::new(ctx.pixels_per_point(), options, [1, 1], vec![]);
    let mut mesh = Mesh::default();
    tessellator.tessellate_border_rect_sides(shape, &mut mesh);
    ui.painter().add(Shape::mesh(mesh));
}
