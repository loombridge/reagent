//! Reagent's icons: strokes drawn in code from a few points each, so they
//! take the theme's colours at any size, light or dark, and need no image
//! files.

use lspk_ui::egui::{Color32, Painter, Rect, Shape, Stroke, pos2, vec2};

/// An icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Undo,
    Redo,
    Previous,
    Next,
    Menu,
    Save,
    Lock,
    Close,
    Plus,
    Search,
}

/// One stroke of an icon, in unit-square points with y down.
#[derive(Clone, Copy)]
enum Part {
    Line(&'static [(f32, f32)]),
    Closed(&'static [(f32, f32)]),
    Circle((f32, f32), f32),
}

impl Icon {
    /// Every icon.
    pub const ALL: [Icon; 10] = [
        Icon::Undo,
        Icon::Redo,
        Icon::Previous,
        Icon::Next,
        Icon::Menu,
        Icon::Save,
        Icon::Lock,
        Icon::Close,
        Icon::Plus,
        Icon::Search,
    ];

    /// What a screen reader says for a button that shows it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Icon::Undo => "Undo",
            Icon::Redo => "Redo",
            Icon::Previous => "Previous",
            Icon::Next => "Next",
            Icon::Menu => "Menu",
            Icon::Save => "Save",
            Icon::Lock => "Lock",
            Icon::Close => "Close",
            Icon::Plus => "Add",
            Icon::Search => "Search",
        }
    }

    fn parts(self) -> &'static [Part] {
        use Part::{Circle, Closed, Line};
        match self {
            Icon::Undo | Icon::Redo => &[
                Line(&[(0.30, 0.18), (0.12, 0.38), (0.30, 0.58)]),
                Line(&[
                    (0.12, 0.38),
                    (0.62, 0.38),
                    (0.86, 0.52),
                    (0.86, 0.68),
                    (0.62, 0.82),
                    (0.34, 0.82),
                ]),
            ],
            Icon::Previous => &[Line(&[(0.62, 0.18), (0.30, 0.50), (0.62, 0.82)])],
            Icon::Next => &[Line(&[(0.38, 0.18), (0.70, 0.50), (0.38, 0.82)])],
            Icon::Menu => &[
                Line(&[(0.16, 0.28), (0.84, 0.28)]),
                Line(&[(0.16, 0.50), (0.84, 0.50)]),
                Line(&[(0.16, 0.72), (0.84, 0.72)]),
            ],
            Icon::Save => &[
                Closed(&[
                    (0.16, 0.16),
                    (0.72, 0.16),
                    (0.84, 0.28),
                    (0.84, 0.84),
                    (0.16, 0.84),
                ]),
                Closed(&[(0.32, 0.16), (0.32, 0.40), (0.66, 0.40), (0.66, 0.16)]),
                Closed(&[(0.30, 0.84), (0.30, 0.62), (0.70, 0.62), (0.70, 0.84)]),
            ],
            Icon::Lock => &[
                Closed(&[(0.20, 0.46), (0.80, 0.46), (0.80, 0.86), (0.20, 0.86)]),
                Line(&[
                    (0.32, 0.46),
                    (0.32, 0.30),
                    (0.40, 0.16),
                    (0.60, 0.16),
                    (0.68, 0.30),
                    (0.68, 0.46),
                ]),
            ],
            Icon::Close => &[
                Line(&[(0.22, 0.22), (0.78, 0.78)]),
                Line(&[(0.78, 0.22), (0.22, 0.78)]),
            ],
            Icon::Plus => &[
                Line(&[(0.50, 0.18), (0.50, 0.82)]),
                Line(&[(0.18, 0.50), (0.82, 0.50)]),
            ],
            Icon::Search => &[
                Circle((0.42, 0.42), 0.26),
                Line(&[(0.61, 0.61), (0.84, 0.84)]),
            ],
        }
    }

    /// The icon drawn to fill `rect` (kept square, centred), its strokes
    /// `width` points wide.
    pub fn paint(self, painter: &Painter, rect: Rect, color: Color32, width: f32) {
        painter.extend(self.shapes(rect, color, width));
    }

    /// The shapes [`paint`](Self::paint) draws.
    #[must_use]
    pub fn shapes(self, rect: Rect, color: Color32, width: f32) -> Vec<Shape> {
        let side = rect.width().min(rect.height());
        let origin = rect.center() - vec2(side, side) / 2.0;
        // Redo is Undo flipped left to right.
        let flip = self == Icon::Redo;
        let at = |&(x, y): &(f32, f32)| {
            let x = if flip { 1.0 - x } else { x };
            pos2(origin.x + x * side, origin.y + y * side)
        };
        let stroke = Stroke::new(width, color);
        self.parts()
            .iter()
            .map(|part| match part {
                Part::Line(points) => Shape::line(points.iter().map(at).collect(), stroke),
                Part::Closed(points) => Shape::closed_line(points.iter().map(at).collect(), stroke),
                Part::Circle(center, radius) => {
                    Shape::circle_stroke(at(center), radius * side, stroke)
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_draws_inside_its_square() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(40.0, 24.0));
        let square = Rect::from_center_size(rect.center(), vec2(24.0, 24.0));
        for icon in Icon::ALL {
            let shapes = icon.shapes(rect, Color32::WHITE, 2.0);
            assert!(!shapes.is_empty(), "{icon:?}");
            for shape in shapes {
                let bounds = shape.visual_bounding_rect();
                assert!(
                    square.expand(0.01).contains_rect(bounds),
                    "{icon:?} {bounds:?}"
                );
            }
            assert!(!icon.name().is_empty());
        }
    }

    #[test]
    fn redo_is_undo_the_other_way_round() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(20.0, 20.0));
        let bounds = |icon: Icon| {
            icon.shapes(rect, Color32::WHITE, 1.0)
                .iter()
                .fold(Rect::NOTHING, |all, shape| {
                    all.union(shape.visual_bounding_rect())
                })
        };
        let (undo, redo) = (bounds(Icon::Undo), bounds(Icon::Redo));
        assert!((undo.left() - (20.0 - redo.right())).abs() < 0.01);
        assert_eq!(undo.height(), redo.height());
    }
}
