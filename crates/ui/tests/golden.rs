//! Snapshots of Reagent's candidate looks: a specimen sheet drawn off
//! screen in each look with its own typeface, compared with the pictures in
//! `tests/golden/`. Remake them with `cargo xtask golden --update`.

use std::sync::Arc;

use lspk_params::{ParamDescriptor, ParamId, ParamUnit};
use lspk_plugin::{Editor, EditorContext, ParamValues};
use lspk_test::snapshot::Suite;
use lspk_ui::egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use lspk_ui::generic::{param_grid, top_bar_height};
use lspk_ui::{EditorOptions, EditorUi, EguiEditor, View};
use reagent_ui::icons::Icon;
use reagent_ui::look::{self, Look};

const GAIN: ParamId = ParamId(1);
const CUTOFF: ParamId = ParamId(2);
const AMOUNT: ParamId = ParamId(3);
const VOICES: ParamId = ParamId(4);
const MODE: ParamId = ParamId(5);
const ON: ParamId = ParamId(6);
const LOCKED: ParamId = ParamId(7);
const DEPTH: ParamId = ParamId(8);

fn values() -> Arc<ParamValues> {
    let values = Arc::new(
        ParamValues::new(vec![
            ParamDescriptor::gain(GAIN, "Level", 12.0, 0.0),
            ParamDescriptor::continuous(
                CUTOFF,
                "Cutoff",
                (20.0, 20_000.0),
                1_200.0,
                ParamUnit::None,
            ),
            ParamDescriptor::continuous(AMOUNT, "Amount", (0.0, 100.0), 50.0, ParamUnit::Percent),
            ParamDescriptor::integer(VOICES, "Voices", (1, 16), 8, ParamUnit::None),
            ParamDescriptor::choice(MODE, "Mode", &["Clean", "Warm", "Hot"], 1),
            ParamDescriptor::toggle(ON, "On", true),
            ParamDescriptor::toggle(LOCKED, "Lock", false),
            ParamDescriptor::continuous(DEPTH, "Depth", (0.0, 100.0), 0.0, ParamUnit::Percent),
        ])
        .expect("valid parameters"),
    );
    values.set(AMOUNT, 80.0);
    values.set(DEPTH, 25.0);
    values
}

/// What a look draws: the bar with its name and the icons, a row of the
/// palette's colours, and controls in their several states.
struct Specimen(String);

impl View for Specimen {
    fn ui(&mut self, ui: &mut Ui, editor: &mut EditorUi<'_>) {
        let theme = editor.theme;
        let (s, c) = (&theme.sizes, &theme.colors);
        let full = ui.max_rect();
        let bar = Rect::from_min_size(full.min, vec2(full.width(), top_bar_height(theme)));
        let painter = ui.painter();
        painter.line_segment(
            [bar.left_bottom(), bar.right_bottom()],
            Stroke::new(1.0, c.separator),
        );
        painter.text(
            pos2(bar.left() + s.spacing, bar.center().y),
            Align2::LEFT_CENTER,
            &self.0,
            theme.font(),
            c.text,
        );
        let icon = s.field_height;
        for (i, kind) in Icon::ALL.iter().rev().enumerate() {
            let at = pos2(
                bar.right() - s.spacing - icon / 2.0 - i as f32 * (icon + 4.0),
                bar.center().y,
            );
            kind.paint(
                painter,
                Rect::from_center_size(at, vec2(icon, icon)),
                c.text_dim,
                s.stroke / 2.0,
            );
        }
        let colors = [
            c.background,
            c.control,
            c.field,
            c.accent,
            c.accent_text,
            c.text,
            c.text_dim,
            c.focus,
            c.separator,
        ];
        let row = bar.bottom() + s.spacing;
        for (i, color) in colors.into_iter().enumerate() {
            let swatch = Rect::from_min_size(
                pos2(full.left() + s.spacing + i as f32 * 34.0, row),
                vec2(28.0, 20.0),
            );
            painter.rect(
                swatch,
                s.corner,
                color,
                Stroke::new(1.0, c.separator),
                lspk_ui::egui::StrokeKind::Inside,
            );
        }
        painter.text(
            pos2(full.right() - s.spacing, row + 10.0),
            Align2::RIGHT_CENTER,
            "Wavetable synthesiser 0123456789",
            theme.small_font(),
            c.text_dim,
        );
        let body = Rect::from_min_max(pos2(full.left(), row + 20.0), full.max);
        let mut grid = ui.new_child(lspk_ui::egui::UiBuilder::new().max_rect(body));
        grid.allocate_exact_size(vec2(0.0, 0.0), Sense::hover());
        param_grid(&mut grid, editor);
    }
}

fn capture(look: &Look, cx: &EditorContext, scale: f32) -> lspk_plugin::EditorImage {
    let mut options = EditorOptions::new(vec2(600.0, 310.0));
    options.theme = look.theme.clone();
    let name = look.name.clone();
    EguiEditor::new(options, move || Specimen(name.clone()))
        .capture(cx, scale)
        .expect("the editor draws")
}

#[test]
fn every_look_matches_its_specimen() {
    let cx = EditorContext::detached(values());
    let mut suite = Suite::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden"));
    for look in look::builtin() {
        suite.add(&look.id, capture(&look, &cx, 1.0));
    }
    suite.assert();
}
