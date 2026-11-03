//! Reagent's look: the tokens (colours, sizes, typeface) a [`Theme`] holds,
//! as three candidate visual identities, each in a dark and a light look,
//! and the theme files a person adds.
//!
//! A theme file is JSON in the `themes` folder of Reagent's user folder
//! ([`appearance::user_dir`](crate::appearance::user_dir)). It names a
//! built-in look to start from and changes what it lists:
//!
//! ```json
//! {
//!   "name": "Dusk",
//!   "base": "orchid-dark",
//!   "colors": { "accent": "#ff8844" },
//!   "sizes": { "knob": 48 },
//!   "font": "Dusk-Regular.ttf"
//! }
//! ```
//!
//! Colours are `#rrggbb`, sizes are points at 100 % zoom, and the font is a
//! TrueType or OpenType file beside the theme file. A file that doesn't read
//! is left out and named, so one mistake never costs the other themes.

use std::path::Path;
use std::sync::Arc;

use lspk_ui::Theme;
use lspk_ui::egui::Color32;
use lspk_ui::fonts::is_font;
use lspk_ui::theme::{Colors, FontSource, Sizes};
use serde::Deserialize;

/// The look Reagent opens with. A placeholder until the maintainer chooses
/// among the candidates.
pub const DEFAULT_LOOK: &str = "orchid-dark";

/// A look: a theme with a name a person picks it by.
#[derive(Clone, Debug)]
pub struct Look {
    /// What a preference file names it by: lower case, `orchid-dark` for a
    /// built-in look, the theme file's name without `.json` for a person's.
    pub id: String,
    /// What a menu shows.
    pub name: String,
    /// The tokens.
    pub theme: Theme,
}

/// One candidate visual identity: a typeface, the shape of its corners and
/// strokes, and a palette each for dark and light.
struct Identity {
    id: &'static str,
    name: &'static str,
    font: &'static [u8],
    corner: f32,
    stroke: f32,
    dark: Colors,
    light: Colors,
}

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// A palette, from the hex values in the order of [`Colors`]' fields:
/// background, control, field, accent, accent text, text, dim text, focus,
/// separator.
fn palette(hex: [u32; 9]) -> Colors {
    let [
        background,
        control,
        field,
        accent,
        accent_text,
        text,
        text_dim,
        focus,
        separator,
    ] = hex.map(rgb);
    Colors {
        background,
        control,
        field,
        accent,
        accent_text,
        text,
        text_dim,
        focus,
        separator,
    }
}

fn identities() -> [Identity; 3] {
    [
        Identity {
            id: "orchid",
            name: "Orchid",
            font: include_bytes!("../assets/fonts/barlow/Barlow-Regular.ttf"),
            corner: 6.0,
            stroke: 3.0,
            dark: palette([
                0x1f1a26, 0x342c3e, 0x15111a, 0xd98bd6, 0x1a1020, 0xece6f0, 0xa89fb3, 0xffd166,
                0x463c52,
            ]),
            light: palette([
                0xf3eef6, 0xdccfe3, 0xffffff, 0x8e3c8a, 0xffffff, 0x231a2a, 0x5d5068, 0x8f5200,
                0xcbbdd4,
            ]),
        },
        Identity {
            id: "lichen",
            name: "Lichen",
            font: include_bytes!("../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"),
            corner: 2.0,
            stroke: 2.5,
            dark: palette([
                0x1c211b, 0x313a2f, 0x131712, 0xb6d457, 0x141a0c, 0xe6eadf, 0xa0a994, 0x7cc7ff,
                0x3f493c,
            ]),
            light: palette([
                0xeef1e6, 0xd3dac4, 0xffffff, 0x4b6a12, 0xffffff, 0x1d2318, 0x566049, 0x0b6fb3,
                0xc1c9b0,
            ]),
        },
        Identity {
            id: "saffron",
            name: "Saffron",
            font: include_bytes!(
                "../assets/fonts/atkinson-hyperlegible/AtkinsonHyperlegible-Regular.ttf"
            ),
            corner: 10.0,
            stroke: 4.0,
            dark: palette([
                0x1b2130, 0x2f384d, 0x121723, 0xf2b53a, 0x1b1405, 0xe9ecf3, 0x9aa4b8, 0x8fe3c0,
                0x3a4560,
            ]),
            light: palette([
                0xf6f0e2, 0xe4d9bd, 0xfffdf6, 0xa45f00, 0xffffff, 0x25201a, 0x6a604e, 0x1b5fa8,
                0xd6c9a8,
            ]),
        },
    ]
}

/// The built-in looks: each candidate identity, dark then light.
#[must_use]
pub fn builtin() -> Vec<Look> {
    let mut looks = Vec::new();
    for identity in identities() {
        let mut theme = Theme::dark();
        theme.sizes.corner = identity.corner;
        theme.sizes.stroke = identity.stroke;
        theme.fonts.ui = FontSource::Bytes(Arc::from(identity.font));
        for (mode, colors) in [("dark", identity.dark), ("light", identity.light)] {
            looks.push(Look {
                id: format!("{}-{mode}", identity.id),
                name: format!("{} {mode}", identity.name),
                theme: Theme {
                    colors,
                    ..theme.clone()
                },
            });
        }
    }
    looks
}

/// The built-in looks and the theme files in `folder`, with a message for
/// each file that isn't listed.
#[must_use]
pub fn load(folder: Option<&Path>) -> (Vec<Look>, Vec<String>) {
    let mut looks = builtin();
    let mut problems = Vec::new();
    let mut files: Vec<_> = folder
        .and_then(|folder| std::fs::read_dir(folder).ok())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    for path in files {
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        let id = path.file_stem().unwrap_or_default().to_string_lossy();
        let id = id.to_lowercase();
        let look = if looks.iter().any(|look| look.id == id) {
            Err("another theme has this file name".to_owned())
        } else {
            std::fs::read_to_string(&path)
                .map_err(|e| format!("can't be read: {e}"))
                .and_then(|text| parse(&id, &text, path.parent().unwrap_or(Path::new("."))))
        };
        match look {
            Ok(look) => looks.push(look),
            Err(message) => problems.push(format!("{file}: {message}")),
        }
    }
    (looks, problems)
}

/// The look called `id` among `looks`, or the default look, or the first
/// if even that is missing.
///
/// # Panics
///
/// If `looks` is empty.
#[must_use]
pub fn choose(mut looks: Vec<Look>, id: &str) -> Look {
    let at = [id, DEFAULT_LOOK]
        .iter()
        .find_map(|id| looks.iter().position(|look| look.id == *id))
        .unwrap_or(0);
    looks.swap_remove(at)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    name: String,
    base: Option<String>,
    #[serde(default)]
    colors: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    sizes: std::collections::BTreeMap<String, f32>,
    font: Option<String>,
}

/// The theme file `text`, from the file named `id`, with its font read from
/// `folder`.
///
/// # Errors
///
/// A message naming what is wrong: JSON that doesn't fit, a base look,
/// colour or size that doesn't exist, a colour that isn't `#rrggbb`, a size
/// that isn't a positive number, or a font that can't be read.
pub fn parse(id: &str, text: &str, folder: &Path) -> Result<Look, String> {
    let file: ThemeFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let base_id = file.base.as_deref().unwrap_or(DEFAULT_LOOK);
    let mut theme = builtin()
        .into_iter()
        .find(|look| look.id == base_id)
        .ok_or_else(|| format!("no look called `{base_id}` to start from"))?
        .theme;
    for (name, value) in &file.colors {
        let slot = color_slots(&mut theme.colors)
            .into_iter()
            .find(|(slot, _)| slot == name)
            .ok_or_else(|| format!("no colour called `{name}`"))?;
        *slot.1 = parse_hex(value).ok_or_else(|| format!("`{value}` isn't a #rrggbb colour"))?;
    }
    for (name, value) in &file.sizes {
        let slot = size_slots(&mut theme.sizes)
            .into_iter()
            .find(|(slot, _)| slot == name)
            .ok_or_else(|| format!("no size called `{name}`"))?;
        if !(value.is_finite() && *value > 0.0 && *value <= 1000.0) {
            return Err(format!("size `{name}` must be between 0 and 1000"));
        }
        *slot.1 = *value;
    }
    if let Some(font) = &file.font {
        let bytes = std::fs::read(folder.join(font))
            .map_err(|e| format!("font `{font}` can't be read: {e}"))?;
        if !is_font(&bytes) {
            return Err(format!("`{font}` isn't a TrueType or OpenType font"));
        }
        theme.fonts.ui = FontSource::Bytes(Arc::from(bytes));
    }
    Ok(Look {
        id: id.to_owned(),
        name: file.name,
        theme,
    })
}

fn parse_hex(text: &str) -> Option<Color32> {
    let hex = text.strip_prefix('#').filter(|hex| hex.len() == 6)?;
    u32::from_str_radix(hex, 16).ok().map(rgb)
}

/// Each colour token by the name a theme file gives it.
fn color_slots(c: &mut Colors) -> [(&'static str, &mut Color32); 9] {
    [
        ("background", &mut c.background),
        ("control", &mut c.control),
        ("field", &mut c.field),
        ("accent", &mut c.accent),
        ("accent_text", &mut c.accent_text),
        ("text", &mut c.text),
        ("text_dim", &mut c.text_dim),
        ("focus", &mut c.focus),
        ("separator", &mut c.separator),
    ]
}

/// Each size token by the name a theme file gives it.
fn size_slots(s: &mut Sizes) -> [(&'static str, &mut f32); 9] {
    [
        ("text", &mut s.text),
        ("small_text", &mut s.small_text),
        ("knob", &mut s.knob),
        ("cell_width", &mut s.cell_width),
        ("cell_height", &mut s.cell_height),
        ("spacing", &mut s.spacing),
        ("field_height", &mut s.field_height),
        ("corner", &mut s.corner),
        ("stroke", &mut s.stroke),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The contrast ratio of two colours, as WCAG 2 defines it.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let luminance = |c: Color32| {
            let [r, g, b] = [c.r(), c.g(), c.b()].map(|v| {
                let v = f32::from(v) / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            });
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn every_built_in_look_is_legible() {
        let looks = builtin();
        assert_eq!(looks.len(), 6);
        for look in &looks {
            let c = &look.theme.colors;
            for ground in [c.background, c.control, c.field] {
                assert!(contrast(c.text, ground) >= 4.5, "{}: text", look.id);
            }
            assert!(
                contrast(c.text_dim, c.background) >= 4.5,
                "{}: dim",
                look.id
            );
            assert!(
                contrast(c.accent_text, c.accent) >= 4.5,
                "{}: on accent",
                look.id
            );
            for shown in [c.accent, c.focus] {
                for ground in [c.background, c.control] {
                    assert!(contrast(shown, ground) >= 3.0, "{}: ring", look.id);
                }
            }
            assert!(matches!(&look.theme.fonts.ui, FontSource::Bytes(b) if is_font(b)));
        }
        let ids: std::collections::BTreeSet<_> = looks.iter().map(|look| &look.id).collect();
        assert_eq!(ids.len(), looks.len());
        assert!(ids.contains(&DEFAULT_LOOK.to_owned()));
    }

    #[test]
    fn a_theme_file_changes_what_it_lists_and_keeps_the_rest() {
        let text = r##"{"name":"Dusk","base":"lichen-light","colors":{"accent":"#ff8844"},"sizes":{"knob":48}}"##;
        let look = parse("dusk", text, Path::new(".")).unwrap();
        let base = builtin()
            .into_iter()
            .find(|l| l.id == "lichen-light")
            .unwrap();
        assert_eq!((look.id.as_str(), look.name.as_str()), ("dusk", "Dusk"));
        assert_eq!(
            look.theme.colors.accent,
            Color32::from_rgb(0xff, 0x88, 0x44)
        );
        assert_eq!(look.theme.colors.text, base.theme.colors.text);
        assert_eq!(look.theme.sizes.knob, 48.0);
        assert_eq!(look.theme.sizes.corner, base.theme.sizes.corner);
    }

    #[test]
    fn a_theme_file_that_is_wrong_says_what_is() {
        let parse = |text: &str| parse("x", text, Path::new(".")).unwrap_err();
        assert!(parse(r#"{"name":"A","base":"nope"}"#).contains("`nope`"));
        assert!(parse(r##"{"name":"A","colors":{"glow":"#ffffff"}}"##).contains("`glow`"));
        assert!(parse(r##"{"name":"A","colors":{"accent":"#fff"}}"##).contains("`#fff`"));
        assert!(parse(r#"{"name":"A","sizes":{"knob":0}}"#).contains("`knob`"));
        assert!(parse(r#"{"name":"A","sizes":{"knob":-4}}"#).contains("`knob`"));
        assert!(parse(r#"{"name":"A","extra":1}"#).contains("extra"));
        assert!(parse(r#"{"name":"A","font":"missing.ttf"}"#).contains("missing.ttf"));
    }

    #[test]
    fn a_theme_files_font_is_read_from_beside_it() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("good.ttf"), lspk_ui::fonts::block_font()).unwrap();
        std::fs::write(folder.path().join("bad.ttf"), b"<html>").unwrap();
        let text = |font: &str| format!(r#"{{"name":"A","font":"{font}"}}"#);
        let look = parse("a", &text("good.ttf"), folder.path()).unwrap();
        assert!(matches!(&look.theme.fonts.ui, FontSource::Bytes(b) if is_font(b)));
        assert!(
            parse("a", &text("bad.ttf"), folder.path())
                .unwrap_err()
                .contains("bad.ttf")
        );
    }

    #[test]
    fn a_folder_of_themes_lists_the_good_and_names_the_rest() {
        let folder = tempfile::tempdir().unwrap();
        let write =
            |file: &str, text: &str| std::fs::write(folder.path().join(file), text).unwrap();
        write("Mine.json", r#"{"name":"Mine"}"#);
        write("broken.json", "{");
        write("orchid-dark.json", r#"{"name":"Impostor"}"#);
        write("notes.txt", "not a theme");
        let (looks, problems) = load(Some(folder.path()));
        assert_eq!(looks.len(), builtin().len() + 1);
        assert_eq!(looks.last().unwrap().id, "mine");
        assert_eq!(problems.len(), 2);
        assert!(problems[0].starts_with("broken.json:"));
        assert!(problems[1].starts_with("orchid-dark.json:"));
        assert_eq!(load(None).0.len(), builtin().len());
    }

    #[test]
    fn a_look_that_is_missing_falls_back_to_the_default() {
        assert_eq!(choose(builtin(), "lichen-light").id, "lichen-light");
        assert_eq!(choose(builtin(), "gone").id, DEFAULT_LOOK);
    }
}
