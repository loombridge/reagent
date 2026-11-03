//! How Reagent's window opens: the look a person chose and the size they
//! made their default, kept in a file of their own, and the size presets
//! the interface's size menu offers.
//!
//! The window resizes by its corner and follows the host's scale factor
//! through the kit's editor; a size here is the zoom it opens at.

use std::io;
use std::path::{Path, PathBuf};

use lspk_ui::EditorOptions;
use lspk_ui::egui::Vec2;
use lspk_ui::theme::clamp_zoom;
use serde::{Deserialize, Serialize};

use crate::look::{DEFAULT_LOOK, Look};

/// The file in Reagent's user folder that holds an [`Appearance`].
pub const APPEARANCE_FILE: &str = "interface.json";

/// The folder in Reagent's user folder that holds theme files.
pub const THEMES_FOLDER: &str = "themes";

/// The sizes the size menu offers, as zoom factors: the reference's
/// sizes the kit's zoom reaches, 50 % to 200 %.
pub const SIZE_PRESETS: [f32; 10] = [0.5, 0.75, 0.9, 1.0, 1.1, 1.25, 1.33, 1.5, 1.75, 2.0];

/// A size as the menu words it: `125%`.
#[must_use]
pub fn size_label(zoom: f32) -> String {
    format!("{:.0}%", clamp_zoom(zoom) * 100.0)
}

/// The size menu's first item, which follows the default size:
/// `Default (100%)`.
#[must_use]
pub fn default_size_label(default_zoom: f32) -> String {
    format!("Default ({})", size_label(default_zoom))
}

/// What a person has chosen about how the window looks and opens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// The chosen look's ID ([`Look::id`]).
    pub look: String,
    /// The zoom the window opens at: the size made the default.
    pub zoom: f32,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            look: DEFAULT_LOOK.to_owned(),
            zoom: 1.0,
        }
    }
}

impl Appearance {
    /// What `folder` holds, or the defaults if there's no file or it can't
    /// be read; the zoom is kept within what the kit can show.
    #[must_use]
    pub fn load(folder: &Path) -> Self {
        let mut saved: Self = std::fs::read(folder.join(APPEARANCE_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        saved.zoom = clamp_zoom(saved.zoom);
        saved
    }

    /// Saves it in `folder`, making the folder if need be, through a
    /// temporary file so a crash never leaves half of one.
    ///
    /// # Errors
    ///
    /// The file couldn't be written.
    pub fn save(&self, folder: &Path) -> io::Result<()> {
        std::fs::create_dir_all(folder)?;
        let path = folder.join(APPEARANCE_FILE);
        let temporary = path.with_extension("json.tmp");
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(self).map_err(io::Error::other)?,
        )?;
        std::fs::rename(temporary, path)
    }
}

/// Reagent's user folder for the plug-in with ID `id`, or `None` if the
/// platform gives no home for it: `~/Library/Application Support/<id>` on
/// macOS, `%APPDATA%\<id>` on Windows, and `$XDG_CONFIG_HOME/<id>` (or
/// `~/.config/<id>`) on Linux, the folder the standalone app keeps its
/// settings in.
#[must_use]
pub fn user_dir(id: &str) -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library/Application Support"))
    } else if cfg!(windows) {
        var("APPDATA")
    } else {
        var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|home| home.join(".config")))
    }?;
    Some(base.join(id))
}

/// Options for an editor that opens in `look`, at the zoom `appearance`
/// holds, with the page shell's `size` and `min_size` in points at 100 %.
#[must_use]
pub fn open_options(
    size: Vec2,
    min_size: Vec2,
    look: &Look,
    appearance: &Appearance,
) -> EditorOptions {
    let mut options = EditorOptions::new(size);
    options.min_size = min_size;
    options.theme = look.theme.clone();
    options.theme.zoom = clamp_zoom(appearance.zoom);
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::builtin;
    use lspk_ui::egui::vec2;

    #[test]
    fn sizes_are_worded_as_the_menu_words_them() {
        assert_eq!(size_label(1.0), "100%");
        assert_eq!(size_label(1.33), "133%");
        assert_eq!(default_size_label(1.25), "Default (125%)");
        assert!(SIZE_PRESETS.contains(&1.0));
        assert!(SIZE_PRESETS.iter().all(|zoom| clamp_zoom(*zoom) == *zoom));
    }

    #[test]
    fn the_chosen_look_and_default_size_are_remembered() {
        let folder = tempfile::tempdir().unwrap();
        assert_eq!(Appearance::load(folder.path()), Appearance::default());
        let chosen = Appearance {
            look: "lichen-light".to_owned(),
            zoom: 1.5,
        };
        // The folder may not exist yet.
        let inner = folder.path().join("com.loombridge.reagent");
        chosen.save(&inner).unwrap();
        assert_eq!(Appearance::load(&inner), chosen);
    }

    #[test]
    fn a_file_that_cant_be_read_or_asks_too_much_is_tamed() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join(APPEARANCE_FILE);
        std::fs::write(&file, "{").unwrap();
        assert_eq!(Appearance::load(folder.path()), Appearance::default());
        std::fs::write(&file, r#"{"zoom": 9}"#).unwrap();
        let loaded = Appearance::load(folder.path());
        assert_eq!(loaded.zoom, lspk_ui::MAX_ZOOM);
        assert_eq!(loaded.look, DEFAULT_LOOK);
    }

    #[test]
    fn the_window_opens_in_the_chosen_look_at_the_default_size() {
        let look = builtin()
            .into_iter()
            .find(|l| l.id == "saffron-light")
            .unwrap();
        let appearance = Appearance {
            look: look.id.clone(),
            zoom: 1.25,
        };
        let options = open_options(vec2(1000.0, 700.0), vec2(500.0, 350.0), &look, &appearance);
        assert_eq!(options.theme.zoom, 1.25);
        assert_eq!(options.theme.colors, look.theme.colors);
        assert_eq!(options.size, vec2(1000.0, 700.0));
        assert_eq!(options.min_size, vec2(500.0, 350.0));
        assert!(options.resizable);
    }
}
