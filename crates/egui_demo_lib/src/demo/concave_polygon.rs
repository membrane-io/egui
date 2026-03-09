use std::f32::consts::TAU;

use egui::{Color32, Frame, Pos2, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, epaint, pos2, vec2};
use epaint::{PathShape, PathStroke};

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
enum ShapeKind {
    Star,
    Arrow,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ConcavePolygonDemo {
    shape_kind: ShapeKind,

    fill_color: Color32,
    stroke_enabled: bool,
    stroke_color: Color32,
    stroke_width: f32,
    stroke_kind: StrokeKind,

    star_points: usize,

    feathering_override_enabled: bool,
    feathering_override: f32,

    use_convex: bool,
    show_wireframe: bool,
}

impl Default for ConcavePolygonDemo {
    fn default() -> Self {
        Self {
            shape_kind: ShapeKind::Star,
            fill_color: Color32::from_rgba_unmultiplied(255, 200, 40, 200),
            stroke_enabled: true,
            stroke_color: Color32::from_rgba_unmultiplied(255, 40, 40, 153),
            stroke_width: 8.0,
            stroke_kind: StrokeKind::Middle,
            star_points: 5,
            feathering_override_enabled: false,
            feathering_override: 1.5,
            use_convex: false,
            show_wireframe: false,
        }
    }
}

fn star_polygon(center: Pos2, outer_r: f32, inner_r: f32, n: usize) -> Vec<Pos2> {
    let mut pts = Vec::with_capacity(n * 2);
    for i in 0..(n * 2) {
        let angle = i as f32 * TAU / (n * 2) as f32 - TAU / 4.0;
        let r = if i % 2 == 0 { outer_r } else { inner_r };
        pts.push(center + r * vec2(angle.cos(), angle.sin()));
    }
    pts
}

fn arrow_polygon(center: Pos2, s: f32) -> Vec<Pos2> {
    let cx = center.x;
    let cy = center.y;
    vec![
        pos2(cx + s * 0.6, cy),
        pos2(cx + s * 0.2, cy),
        pos2(cx + s * 0.2, cy + s),
        pos2(cx - s * 0.2, cy + s),
        pos2(cx - s * 0.2, cy),
        pos2(cx - s * 0.6, cy),
        pos2(cx, cy - s),
    ]
}

impl ConcavePolygonDemo {
    fn build_points(&self, center: Pos2, size: f32, time: f64) -> Vec<Pos2> {
        match self.shape_kind {
            ShapeKind::Star => star_polygon(center, size, size * 0.4, self.star_points),
            ShapeKind::Arrow => arrow_polygon(center, size),
        }
    }

    fn build_stroke(&self) -> PathStroke {
        if self.stroke_enabled {
            PathStroke {
                width: self.stroke_width,
                color: epaint::ColorMode::Solid(self.stroke_color),
                kind: self.stroke_kind,
            }
        } else {
            PathStroke::NONE
        }
    }

    fn render_controls(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Shape:");
            ui.radio_value(&mut self.shape_kind, ShapeKind::Star, "Star");
            ui.radio_value(&mut self.shape_kind, ShapeKind::Arrow, "Arrow");
        });

        ui.separator();

        egui::Grid::new("concave_controls")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Fill color:");
                ui.color_edit_button_srgba(&mut self.fill_color);
                ui.end_row();

                ui.label("Stroke:");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.stroke_enabled, "Enabled");
                    if self.stroke_enabled {
                        ui.color_edit_button_srgba(&mut self.stroke_color);
                    }
                });
                ui.end_row();

                if self.stroke_enabled {
                    ui.label("Stroke width:");
                    ui.add(egui::Slider::new(&mut self.stroke_width, 0.0..=50.0).suffix("px"));
                    ui.end_row();

                    ui.label("Stroke kind:");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.stroke_kind, StrokeKind::Inside, "Inside");
                        ui.radio_value(&mut self.stroke_kind, StrokeKind::Middle, "Middle");
                        ui.radio_value(&mut self.stroke_kind, StrokeKind::Outside, "Outside");
                    });
                    ui.end_row();
                }

                if self.shape_kind == ShapeKind::Star {
                    ui.label("Star points:");
                    ui.add(egui::Slider::new(&mut self.star_points, 3..=20));
                    ui.end_row();
                }

                ui.label("Feathering override:");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.feathering_override_enabled, "");
                    if self.feathering_override_enabled {
                        ui.add(
                            egui::Slider::new(&mut self.feathering_override, 0.0..=50.0)
                                .suffix("px"),
                        );
                    }
                });
                ui.end_row();

                ui.label("Debug:");
                ui.vertical(|ui| {
                    ui.checkbox(&mut self.show_wireframe, "Show wireframe");
                    ui.checkbox(&mut self.use_convex, "Use convex shape tessellatoro");
                });
                ui.end_row();
            });
    }

    fn render_canvas(&self, ui: &mut Ui) {
        Frame::canvas(ui.style()).show(ui, |ui| {
            let avail = ui.available_size();
            let size = avail.min_elem() * 0.35;
            let (resp, painter) = ui.allocate_painter(avail, Sense::hover());
            let center = resp.rect.center();

            let time = ui.input(|i| i.time);

            let points = self.build_points(center, size, time);
            let stroke = self.build_stroke();

            if self.feathering_override_enabled {
                let mut mesh = epaint::Mesh::default();
                let pixels_per_point = ui.ctx().pixels_per_point();
                let mut tessellator = epaint::Tessellator::new(
                    pixels_per_point,
                    epaint::TessellationOptions {
                        feathering: true,
                        feathering_size_in_pixels: self.feathering_override * pixels_per_point,
                        ..Default::default()
                    },
                    ui.fonts(|f| f.font_image_size()),
                    vec![],
                );
                let shape = if self.use_convex {
                    PathShape::convex_polygon(points.clone(), self.fill_color, stroke.clone())
                } else {
                    PathShape::polygon(points.clone(), self.fill_color, stroke.clone())
                };
                tessellator.tessellate_path(&shape, &mut mesh);
                painter.add(Shape::mesh(std::sync::Arc::new(mesh)));
            } else {
                let shape = if self.use_convex {
                    PathShape::convex_polygon(points.clone(), self.fill_color, stroke.clone())
                } else {
                    PathShape::polygon(points.clone(), self.fill_color, stroke.clone())
                };
                painter.add(Shape::Path(shape));
            }

            if self.show_wireframe {
                let shape = PathShape::polygon(points.clone(), self.fill_color, stroke.clone());
                let mut mesh = epaint::Mesh::default();
                let pixels_per_point = ui.ctx().pixels_per_point();
                let mut tessellator = epaint::Tessellator::new(
                    pixels_per_point,
                    Default::default(),
                    ui.fonts(|f| f.font_image_size()),
                    vec![],
                );
                tessellator.tessellate_path(&shape, &mut mesh);

                let edge_stroke =
                    Stroke::new(0.5, Color32::from_rgba_unmultiplied(255, 255, 255, 120));
                let tri_count = mesh.indices.len() / 3;
                let vert_count = mesh.vertices.len();
                for tri in mesh.indices.chunks_exact(3) {
                    let a = mesh.vertices[tri[0] as usize].pos;
                    let b = mesh.vertices[tri[1] as usize].pos;
                    let c = mesh.vertices[tri[2] as usize].pos;
                    painter.line_segment([a, b], edge_stroke);
                    painter.line_segment([b, c], edge_stroke);
                    painter.line_segment([c, a], edge_stroke);
                }

                let label_pos = resp.rect.left_bottom() + vec2(4.0, -4.0);
                painter.text(
                    label_pos,
                    egui::Align2::LEFT_BOTTOM,
                    format!("{vert_count} verts, {tri_count} tris"),
                    egui::FontId::monospace(11.0),
                    Color32::WHITE,
                );
            }
        });
    }
}

impl super::Demo for ConcavePolygonDemo {
    fn name(&self) -> &'static str {
        "🔷 Concave Polygon"
    }

    fn show(&mut self, ui: &mut Ui, open: &mut bool) {
        use super::View as _;
        egui::Window::new(self.name())
            .open(open)
            .default_width(480.0)
            .show(ui.ctx(), |ui| {
                self.ui(ui);
            });
    }
}

impl super::View for ConcavePolygonDemo {
    fn ui(&mut self, ui: &mut Ui) {
        self.render_controls(ui);
        ui.separator();
        self.render_canvas(ui);
    }
}
