//! MEMBRANE: tessellation of [`BorderRectShape`].
//!
//! The mesh has four vertices for each point of the outline, as the thick stroke of
//! [`Tessellator::tessellate_rect`] has:
//!
//! ```text
//! v0   transparent, half a feather outside of the outer edge
//! v1   stroke color, half a feather inside of the outer edge
//! v2   stroke color, half a feather outside of the inner edge
//! v3   fill color, half a feather inside of the inner edge
//! ```
//!
//! The outer edge and the inner edge have one point for each point of the outline, so a strip of
//! triangles connects them. The inner point is the outer point minus the normal of the outline,
//! with the width of the left or right side on the x axis and the width of the top or bottom side
//! on the y axis. For four equal sides, this is the calculation of `tessellate_rect`. On a round
//! corner, the inner edge is an ellipse with the outer radius minus the width of each side, as in
//! CSS.

use core::f32::consts::{FRAC_PI_2, TAU};

use emath::{GuiRounding as _, NumExt as _, Pos2, Rect, Vec2, vec2};

use super::{Tessellator, mul_color, path};
use crate::{BorderRectShape, Color32, CornerRadiusF32, MarginF32, Mesh, RectShape, SideColors};

/// A point of the outer edge of the outline.
#[derive(Clone, Copy, Debug)]
struct OutlinePoint {
    outer: Pos2,

    /// The stroke width of the left or right side on the x axis, and of the top or bottom side on
    /// the y axis.
    widths: Vec2,

    color: Color32,

    /// The stroke width at this point, for a point of a sharp corner. A sharp corner has one point
    /// for each of its two sides, with the same position. If `None`, the tessellator calculates the
    /// width from the distance between the two edges.
    width: Option<f32>,
}

/// One corner of the rectangle, in the order of the outline.
struct Corner {
    pos: Pos2,

    /// The direction from [`Self::pos`] into the rectangle, on each axis.
    inward: Vec2,

    /// The quadrant of [`path::add_circle_quadrant`].
    quadrant: f32,

    /// The radius of the outer edge.
    radius: f32,

    /// The stroke width of the left or the right side.
    width_x: f32,

    /// The stroke width of the top or the bottom side.
    width_y: f32,

    /// The stroke width and the color of the side before the corner, in the order of the outline.
    width_before: f32,
    color_before: Color32,

    /// The stroke width and the color of the side after the corner.
    width_after: f32,
    color_after: Color32,
}

impl Tessellator {
    /// Tessellate a single [`BorderRectShape`] into a [`Mesh`].
    ///
    /// If all four sides have the same stroke, this calls [`Self::tessellate_rect`].
    pub fn tessellate_border_rect(&mut self, shape: &BorderRectShape, out: &mut Mesh) {
        if let Some(rect_shape) = shape.as_rect_shape() {
            self.tessellate_rect(&rect_shape, out);
        } else {
            self.tessellate_border_rect_sides(shape, out);
        }
    }

    /// Tessellate a [`BorderRectShape`] with a stroke for each side, also when all sides are equal.
    ///
    /// Use [`Self::tessellate_border_rect`] to paint. The demo and the tests use this function to
    /// compare the result with [`Self::tessellate_rect`].
    pub fn tessellate_border_rect_sides(&mut self, shape: &BorderRectShape, out: &mut Mesh) {
        if self.options.coarse_tessellation_culling
            && !shape.visual_bounding_rect().intersects(self.clip_rect)
        {
            return;
        }

        let BorderRectShape {
            mut rect,
            corner_radius,
            fill,
            stroke_width: w,
            stroke_color,
        } = *shape;

        // The fill covers the whole rectangle and each side paints over it, as CSS paints the
        // background under the border. Each stroke vertex gets the blended color, so the mesh has
        // no overlap and the outer feather shows no fill.
        let c = stroke_color.over(fill);

        // It is common to create a rectangle with an infinite size. `tessellate_rect` does the same.
        rect.min = rect.min.at_least(emath::pos2(-1e7, -1e7));
        rect.max = rect.max.at_most(emath::pos2(1e7, 1e7));

        let feathering = self.feathering;

        // If the stroke covers the whole rectangle, paint a fill in the color of the widest side.
        // `tessellate_rect` does the same for one stroke width.
        if rect.width() <= w.left + w.right + 0.5 * feathering
            || rect.height() <= w.top + w.bottom + 0.5 * feathering
        {
            let sides = [
                (w.left, c.left),
                (w.right, c.right),
                (w.top, c.top),
                (w.bottom, c.bottom),
            ];
            let (_, color) =
                sides
                    .into_iter()
                    .fold((f32::MIN, Color32::TRANSPARENT), |widest, side| {
                        if widest.0 < side.0 { side } else { widest }
                    });
            self.tessellate_rect(&RectShape::filled(rect, corner_radius, color), out);
            return;
        }

        // `tessellate_rect` puts the outside of an inside stroke on a pixel boundary.
        if self.options.round_rects_to_pixels {
            rect = rect.round_to_pixels(self.pixels_per_point);
        }

        let corners = corners(rect, corner_radius, w, c);
        let mut points = Vec::with_capacity(4 * 36);
        for corner in &corners {
            add_corner_points(&mut points, corner);
        }

        let normals = outline_normals(&points);

        let n = points.len() as u32;
        out.reserve_vertices(4 * n as usize);
        out.reserve_triangles(6 * n as usize + n as usize);
        let idx = out.vertices.len() as u32;

        for (point, &normal) in points.iter().zip(&normals) {
            let o = point.outer;
            let i = o - point.widths * normal;
            let width = point.width.unwrap_or_else(|| {
                if normal == Vec2::ZERO {
                    0.0
                } else {
                    ((o - i).dot(normal) / normal.length_sq()).at_least(0.0)
                }
            });

            // The same threshold as the thin line of `stroke_and_fill_path`.
            if 0.0 < feathering && width <= 0.9 * feathering {
                // Paint a ridge two feathers wide, and reduce the opacity instead of the width.
                // If there is a fill, move the ridge out so that the fill keeps its area. This is
                // the calculation of `stroke_and_fill_path`, relative to the inner point. On a
                // straight side the inner point is `o - width * normal`. At a sharp corner the
                // inner point is the corner of the inner edge, so the fill stays convex.
                let opacity = width / feathering;
                let p = if fill == Color32::TRANSPARENT {
                    i + 0.5 * width * normal
                } else {
                    i + 0.5 * feathering * normal
                };
                let color = mul_color(point.color, opacity);
                out.colored_vertex(p + feathering * normal, Color32::TRANSPARENT);
                out.colored_vertex(p, color);
                out.colored_vertex(p, color);
                out.colored_vertex(p - feathering * normal, fill);
            } else {
                let half = 0.5 * feathering;
                out.colored_vertex(o + half * normal, Color32::TRANSPARENT);
                out.colored_vertex(o - half * normal, point.color);
                out.colored_vertex(i + half * normal, point.color);
                out.colored_vertex(i - half * normal, fill);
            }
        }

        let mut i0 = n - 1;
        for i1 in 0..n {
            for band in 0..3 {
                out.add_triangle(
                    idx + 4 * i0 + band,
                    idx + 4 * i0 + band + 1,
                    idx + 4 * i1 + band,
                );
                out.add_triangle(
                    idx + 4 * i0 + band + 1,
                    idx + 4 * i1 + band,
                    idx + 4 * i1 + band + 1,
                );
            }
            i0 = i1;
        }

        // The inner edge is convex, so a fan of triangles fills it.
        if fill != Color32::TRANSPARENT {
            for i in 2..n {
                out.add_triangle(idx + 4 * (i - 1) + 3, idx + 3, idx + 4 * i + 3);
            }
        }
    }
}

/// The four corners in the order of [`path::rounded_rectangle`]: south east, south west, north west
/// and north east.
///
/// `c` holds the color of each side, blended over the fill.
fn corners(
    rect: Rect,
    corner_radius: crate::CornerRadius,
    w: MarginF32,
    c: SideColors,
) -> [Corner; 4] {
    let original = CornerRadiusF32::from(corner_radius);

    // `tessellate_rect` makes the same change to the radius of a corner. A radius that is smaller
    // than the stroke width becomes larger, so that the inner edge does not cross itself.
    let max_radius = 0.5 * rect.width().min(rect.height());
    let radius = |original: f32, width_x: f32, width_y: f32| {
        if original == 0.0 {
            0.0
        } else {
            (original + 0.4)
                .at_least(width_x.max(width_y) + 0.1)
                .at_most(max_radius)
        }
    };

    let (min, max) = (rect.min, rect.max);
    [
        Corner {
            pos: max,
            inward: vec2(-1.0, -1.0),
            quadrant: 0.0,
            radius: radius(original.se, w.right, w.bottom),
            width_x: w.right,
            width_y: w.bottom,
            width_before: w.right,
            color_before: c.right,
            width_after: w.bottom,
            color_after: c.bottom,
        },
        Corner {
            pos: emath::pos2(min.x, max.y),
            inward: vec2(1.0, -1.0),
            quadrant: 1.0,
            radius: radius(original.sw, w.left, w.bottom),
            width_x: w.left,
            width_y: w.bottom,
            width_before: w.bottom,
            color_before: c.bottom,
            width_after: w.left,
            color_after: c.left,
        },
        Corner {
            pos: min,
            inward: vec2(1.0, 1.0),
            quadrant: 2.0,
            radius: radius(original.nw, w.left, w.top),
            width_x: w.left,
            width_y: w.top,
            width_before: w.left,
            color_before: c.left,
            width_after: w.top,
            color_after: c.top,
        },
        Corner {
            pos: emath::pos2(max.x, min.y),
            inward: vec2(-1.0, 1.0),
            quadrant: 3.0,
            radius: radius(original.ne, w.right, w.top),
            width_x: w.right,
            width_y: w.top,
            width_before: w.top,
            color_before: c.top,
            width_after: w.right,
            color_after: c.right,
        },
    ]
}

fn add_corner_points(points: &mut Vec<OutlinePoint>, corner: &Corner) {
    let r = corner.radius;
    let widths = vec2(corner.width_x, corner.width_y);
    if r <= 0.0 {
        // A sharp corner has one point for the side before it and one point for the side after it.
        points.push(OutlinePoint {
            outer: corner.pos,
            widths,
            color: corner.color_before,
            width: Some(corner.width_before),
        });
        points.push(OutlinePoint {
            outer: corner.pos,
            widths,
            color: corner.color_after,
            width: Some(corner.width_after),
        });
        return;
    }

    let center = corner.pos + corner.inward * r;
    let point = |outer: Pos2, color: Color32| OutlinePoint {
        outer,
        widths,
        color,
        width: None,
    };

    // Use the same points as `path::rounded_rectangle`, so that the outer edge is identical.
    let mut arc = Vec::new();
    path::add_circle_quadrant(&mut arc, center, r, corner.quadrant);

    // As in CSS, the color changes on the line from the outer corner to the inner corner. The
    // angle of that line is relative to the start of the quadrant.
    let split = corner.width_before.atan2(corner.width_after);
    let start = corner.quadrant * FRAC_PI_2;
    let mut split_done = corner.color_before == corner.color_after;
    for pos in arc {
        let normal = (pos - center) / r;
        let mut angle = normal.y.atan2(normal.x) - start;
        if angle < -0.1 {
            angle += TAU;
        }
        if !split_done && split <= angle {
            let split_pos = center + r * Vec2::angled(start + split);
            points.push(point(split_pos, corner.color_before));
            points.push(point(split_pos, corner.color_after));
            split_done = true;
        }
        let color = if split_done {
            corner.color_after
        } else {
            corner.color_before
        };
        points.push(point(pos, color));
    }
}

/// The normal of each point of a closed outline, as [`super::Path::add_line_loop`] calculates it.
///
/// The outline can hold a point more than once, for example where a corner is sharp or a side is
/// fully round. Each copy of a point receives the normal of the two edges next to it.
fn outline_normals(points: &[OutlinePoint]) -> Vec<Vec2> {
    let n = points.len();
    let pos = |point: &OutlinePoint| point.outer;
    let same = |a: Pos2, b: Pos2| (a - b).length_sq() < 1e-6;
    (0..n)
        .map(|i| {
            let p = pos(&points[i]);
            let prev = (1..n)
                .map(|k| pos(&points[(i + n - k) % n]))
                .find(|&q| !same(p, q));
            let next = (1..n)
                .map(|k| pos(&points[(i + k) % n]))
                .find(|&q| !same(p, q));
            let (Some(prev), Some(next)) = (prev, next) else {
                return Vec2::ZERO;
            };
            let n0 = (p - prev).normalized().rot90();
            let n1 = (next - p).normalized().rot90();
            let normal = (n0 + n1) / 2.0;
            let length_sq = normal.length_sq();
            if length_sq < 1e-6 {
                n0
            } else {
                normal / length_sq
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use emath::{Pos2, Rect, pos2, vec2};

    use crate::{
        BorderRectShape, Color32, CornerRadius, MarginF32, Mesh, RectShape, SideColors, Stroke,
        StrokeKind, TessellationOptions, Tessellator,
    };

    fn cross(a: emath::Vec2, b: emath::Vec2) -> f32 {
        a.x * b.y - a.y * b.x
    }

    /// Paint a mesh on a grid of physical pixels. Each pixel takes the color of the mesh at its
    /// center, as the GPU calculates it. The result holds premultiplied colors.
    fn rasterize(mesh: &Mesh, pixels_per_point: f32, size: [usize; 2]) -> Vec<[f32; 4]> {
        let mut image = vec![[0.0; 4]; size[0] * size[1]];
        for triangle in mesh.indices.chunks_exact(3) {
            let [a, b, c] = [0, 1, 2].map(|k| mesh.vertices[triangle[k] as usize]);
            let area = cross(b.pos - a.pos, c.pos - a.pos);
            if area.abs() < 1e-9 {
                continue;
            }
            let bounds = Rect::from_points(&[a.pos, b.pos, c.pos]);
            let x_range = ((bounds.min.x * pixels_per_point).floor().max(0.0) as usize)
                ..((bounds.max.x * pixels_per_point).ceil() as usize).min(size[0]);
            let y_range = ((bounds.min.y * pixels_per_point).floor().max(0.0) as usize)
                ..((bounds.max.y * pixels_per_point).ceil() as usize).min(size[1]);
            for y in y_range {
                for x in x_range.clone() {
                    // Move the sample a little, so that it does not fall on a shared edge of two
                    // triangles. The GPU has a rule for such a sample, and this function has not.
                    let p = pos2(x as f32 + 0.5013, y as f32 + 0.5007) / pixels_per_point;
                    let wa = cross(b.pos - p, c.pos - p) / area;
                    let wb = cross(c.pos - p, a.pos - p) / area;
                    let wc = 1.0 - wa - wb;
                    if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                        continue;
                    }
                    let src = [0, 1, 2, 3].map(|k| {
                        let channel = |color: Color32| color.to_array()[k] as f32 / 255.0;
                        wa * channel(a.color) + wb * channel(b.color) + wc * channel(c.color)
                    });
                    let dst = &mut image[y * size[0] + x];
                    for k in 0..4 {
                        dst[k] = src[k] + dst[k] * (1.0 - src[3]);
                    }
                }
            }
        }
        image
    }

    fn tessellator(pixels_per_point: f32) -> Tessellator {
        Tessellator::new(
            pixels_per_point,
            TessellationOptions::default(),
            [1, 1],
            vec![],
        )
    }

    fn largest_difference(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
        a.iter()
            .zip(b)
            .flat_map(|(a, b)| (0..4).map(move |k| (a[k] - b[k]).abs()))
            .fold(0.0, f32::max)
    }

    /// The mesh for four equal sides must paint the same pixels as `tessellate_rect`. The two
    /// meshes do not have the same vertices: the side mesh has no line for a thin rectangle, and
    /// it has two copies of each sharp corner.
    #[test]
    fn equal_sides_paint_the_same_pixels_as_rect_shape() {
        let rect = Rect::from_min_size(pos2(3.3, 4.7), vec2(70.0, 44.0));
        let image_size = [240, 160];
        let mut worst = (0.0, String::new());
        for pixels_per_point in [1.0, 1.5, 2.0, 3.0] {
            for radius in [0, 1, 2, 3, 4, 6, 8, 12, 17, 22, 40] {
                for width in [0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 7.0] {
                    for fill in [Color32::TRANSPARENT, Color32::from_gray(90)] {
                        for color in [
                            Color32::from_rgb(230, 120, 30),
                            Color32::from_white_alpha(60),
                            Color32::TRANSPARENT,
                        ] {
                            let rect_shape = RectShape::new(
                                rect,
                                radius,
                                fill,
                                // The stroke of `RectShape` paints over the fill, as CSS does.
                                Stroke::new(width, fill.blend(color)),
                                StrokeKind::Inside,
                            );
                            let border_shape = BorderRectShape::new(
                                rect,
                                radius,
                                fill,
                                MarginF32::same(width),
                                color,
                            );

                            let mut expected = Mesh::default();
                            tessellator(pixels_per_point)
                                .tessellate_rect(&rect_shape, &mut expected);
                            let mut actual = Mesh::default();
                            tessellator(pixels_per_point)
                                .tessellate_border_rect_sides(&border_shape, &mut actual);
                            assert!(actual.is_valid());

                            let difference = largest_difference(
                                &rasterize(&expected, pixels_per_point, image_size),
                                &rasterize(&actual, pixels_per_point, image_size),
                            );
                            if worst.0 < difference {
                                worst = (
                                    difference,
                                    format!(
                                        "pixels_per_point {pixels_per_point}, radius {radius}, \
                                         width {width}, fill {fill:?}, color {color:?}"
                                    ),
                                );
                            }
                        }
                    }
                }
            }
        }
        // The two meshes differ by the rounding of one color level at most.
        assert!(
            worst.0 <= 1.5 / 255.0,
            "largest difference {} for {}",
            worst.0,
            worst.1
        );
    }

    fn pixel(image: &[[f32; 4]], width: usize, pos: Pos2) -> [u8; 4] {
        image[pos.y as usize * width + pos.x as usize].map(|v| (v * 255.0).round() as u8)
    }

    #[test]
    fn each_side_paints_its_own_width_and_color() {
        let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(60.0, 40.0));
        let colors = SideColors {
            left: Color32::from_rgb(255, 0, 0),
            right: Color32::from_rgb(0, 255, 0),
            top: Color32::TRANSPARENT,
            bottom: Color32::from_rgb(0, 0, 255),
        };
        let widths = MarginF32 {
            left: 2.0,
            right: 4.0,
            top: 0.0,
            bottom: 1.0,
        };
        let fill = Color32::from_gray(50);
        for radius in [0, 8] {
            let shape = BorderRectShape::new(rect, radius, fill, widths, colors);
            let mut mesh = Mesh::default();
            tessellator(1.0).tessellate_border_rect(&shape, &mut mesh);
            assert!(mesh.is_valid());
            assert!(
                mesh.vertices
                    .iter()
                    .all(|v| v.pos.x.is_finite() && v.pos.y.is_finite())
            );
            let image = rasterize(&mesh, 1.0, [80, 60]);
            let at = |x: f32, y: f32| pixel(&image, 80, pos2(x, y));
            let fill = fill.to_array();

            // The middle of each side.
            assert_eq!(at(10.0, 30.0), colors.left.to_array(), "radius {radius}");
            assert_eq!(at(11.0, 30.0), colors.left.to_array(), "radius {radius}");
            assert_eq!(at(12.0, 30.0), fill, "radius {radius}");
            assert_eq!(at(69.0, 30.0), colors.right.to_array(), "radius {radius}");
            assert_eq!(at(66.0, 30.0), colors.right.to_array(), "radius {radius}");
            assert_eq!(at(65.0, 30.0), fill, "radius {radius}");
            assert_eq!(at(40.0, 10.0), fill, "radius {radius}");
            assert_eq!(at(40.0, 49.0), colors.bottom.to_array(), "radius {radius}");
            assert_eq!(at(40.0, 48.0), fill, "radius {radius}");
            assert_eq!(at(40.0, 50.0), [0; 4], "radius {radius}");
        }
    }

    #[test]
    fn a_side_with_no_width_paints_only_the_fill() {
        // A box with only a bottom side and round corners: the top half looks like a filled rect.
        let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(60.0, 40.0));
        let fill = Color32::from_gray(50);
        let widths = MarginF32 {
            left: 0.0,
            right: 0.0,
            top: 0.0,
            bottom: 1.0,
        };
        let shape = BorderRectShape::new(rect, 8, fill, widths, Color32::WHITE);
        let mut actual = Mesh::default();
        tessellator(2.0).tessellate_border_rect(&shape, &mut actual);
        let mut expected = Mesh::default();
        tessellator(2.0).tessellate_rect(&RectShape::filled(rect, 8, fill), &mut expected);

        let size = [160, 120];
        let actual = rasterize(&actual, 2.0, size);
        let expected = rasterize(&expected, 2.0, size);
        // The upper half of the two images, above the corners of the bottom side.
        let half = size[0] * 2 * 30;
        let difference = largest_difference(&actual[..half], &expected[..half]);
        assert!(difference <= 0.1, "largest difference {difference}");
    }

    #[test]
    fn the_fill_shows_under_a_translucent_side() {
        // The bottom side is 4 wide. Its area shows the bottom color over the fill, as in CSS.
        let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(60.0, 40.0));
        let fill = Color32::from_gray(50);
        for bottom in [Color32::TRANSPARENT, Color32::from_white_alpha(100)] {
            let shape = BorderRectShape::new(
                rect,
                8,
                fill,
                MarginF32::same(4.0),
                SideColors {
                    bottom,
                    ..SideColors::same(Color32::WHITE)
                },
            );
            let mut actual = Mesh::default();
            tessellator(2.0).tessellate_border_rect(&shape, &mut actual);
            let mut expected = Mesh::default();
            tessellator(2.0).tessellate_rect(
                &RectShape::filled(rect, 8, fill.blend(bottom)),
                &mut expected,
            );

            let size = [160, 120];
            let actual = rasterize(&actual, 2.0, size);
            let expected = rasterize(&expected, 2.0, size);
            // The rows of the bottom side below its inner feather, between the corners of the
            // left and the right side.
            let row = |y: usize| {
                let start = y * size[0] + 2 * 30;
                start..start + 2 * 20
            };
            for y in 2 * 46 + 1..2 * 50 {
                let difference = largest_difference(&actual[row(y)], &expected[row(y)]);
                assert!(
                    difference <= 0.1,
                    "bottom {bottom:?}, row {y}: largest difference {difference}"
                );
            }
        }
    }

    #[test]
    fn border_rect_scales_with_the_layer() {
        let mut shape: crate::Shape = BorderRectShape::new(
            Rect::from_min_size(pos2(1.0, 2.0), vec2(10.0, 20.0)),
            CornerRadius::same(4),
            Color32::WHITE,
            MarginF32 {
                left: 1.0,
                right: 0.0,
                top: 2.0,
                bottom: 0.5,
            },
            Color32::BLACK,
        )
        .into();
        shape.scale(2.0);
        let crate::Shape::BorderRect(shape) = shape else {
            panic!("the shape changed its type");
        };
        assert_eq!(
            shape.rect,
            Rect::from_min_size(pos2(2.0, 4.0), vec2(20.0, 40.0))
        );
        assert_eq!(shape.corner_radius, CornerRadius::same(8));
        assert_eq!(
            shape.stroke_width,
            MarginF32 {
                left: 2.0,
                right: 0.0,
                top: 4.0,
                bottom: 1.0
            }
        );
    }
}
