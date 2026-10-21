use crate::*;

/// MEMBRANE: a rectangle with a different stroke width and color on each side, like a CSS border.
///
/// The stroke is always on the inside of [`Self::rect`], as with [`StrokeKind::Inside`].
/// The fill covers only the area inside the stroke.
///
/// When all four sides have the same width and color, the tessellator paints a [`RectShape`],
/// so the result is identical to [`RectShape`] with [`StrokeKind::Inside`].
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct BorderRectShape {
    pub rect: Rect,

    /// The outer corner radii of the rectangle.
    ///
    /// The inner corner of each side is an ellipse with the outer radius minus the width
    /// of each adjacent side, as in CSS.
    pub corner_radius: CornerRadius,

    /// The color of the area inside the stroke.
    pub fill: Color32,

    /// The stroke width of each side.
    pub stroke_width: MarginF32,

    /// The stroke color of each side.
    pub stroke_color: SideColors,
}

/// MEMBRANE: one color for each side of a rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct SideColors {
    pub left: Color32,
    pub right: Color32,
    pub top: Color32,
    pub bottom: Color32,
}

impl SideColors {
    #[inline]
    pub const fn same(color: Color32) -> Self {
        Self {
            left: color,
            right: color,
            top: color,
            bottom: color,
        }
    }
}

impl From<Color32> for SideColors {
    #[inline]
    fn from(color: Color32) -> Self {
        Self::same(color)
    }
}

#[test]
fn border_rect_shape_size() {
    assert!(
        core::mem::size_of::<BorderRectShape>() <= core::mem::size_of::<RectShape>(),
        "BorderRectShape must not make Shape larger"
    );
}

impl BorderRectShape {
    #[inline]
    pub fn new(
        rect: Rect,
        corner_radius: impl Into<CornerRadius>,
        fill: impl Into<Color32>,
        stroke_width: impl Into<MarginF32>,
        stroke_color: impl Into<SideColors>,
    ) -> Self {
        Self {
            rect,
            corner_radius: corner_radius.into(),
            fill: fill.into(),
            stroke_width: stroke_width.into(),
            stroke_color: stroke_color.into(),
        }
    }

    /// The [`RectShape`] that paints the same pixels, if all four sides have the same stroke.
    pub fn as_rect_shape(&self) -> Option<RectShape> {
        let Self {
            rect,
            corner_radius,
            fill,
            stroke_width: w,
            stroke_color: c,
        } = *self;
        let same_width = w.left == w.right && w.left == w.top && w.left == w.bottom;
        if !same_width {
            return None;
        }
        // The color of a side with no width does not matter.
        let same_color = c.left == c.right && c.left == c.top && c.left == c.bottom;
        if w.left > 0.0 && !same_color {
            return None;
        }
        let stroke = if w.left > 0.0 {
            Stroke::new(w.left, c.left)
        } else {
            Stroke::NONE
        };
        Some(RectShape::new(
            rect,
            corner_radius,
            fill,
            stroke,
            StrokeKind::Inside,
        ))
    }

    /// The visual bounding rectangle. The stroke is inside the rectangle, so this is [`Self::rect`].
    pub fn visual_bounding_rect(&self) -> Rect {
        let w = self.stroke_width;
        let c = self.stroke_color;
        let side_visible =
            |width: f32, color: Color32| width > 0.0 && color != Color32::TRANSPARENT;
        let stroke_visible = side_visible(w.left, c.left)
            || side_visible(w.right, c.right)
            || side_visible(w.top, c.top)
            || side_visible(w.bottom, c.bottom);
        if self.fill == Color32::TRANSPARENT && !stroke_visible {
            Rect::NOTHING
        } else {
            self.rect
        }
    }
}

impl From<BorderRectShape> for Shape {
    #[inline(always)]
    fn from(shape: BorderRectShape) -> Self {
        Self::BorderRect(shape)
    }
}
