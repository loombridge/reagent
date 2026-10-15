//! What a fuzz target does with bytes: feed Reagent's preset reader, and
//! check what it accepts keeps the property the reader promises.
//!
//! The reader is [`reagent::preset::decode`]: the plug-in kit's format
//! (schema version, migrations, sparse values, symbolic enums), then
//! Reagent's own state in it (the modulation matrix, each oscillator slot's
//! engines). Whatever it accepts, this checks again:
//!
//! - **No panic.** Every byte string is refused with an error or read as a
//!   preset; nothing unwinds.
//! - **Anything the reader accepts saves again and loads back as the same
//!   patch.** The preset is applied to fresh parameter values, saved, and
//!   read again: the second read equals the first.

use std::sync::{Arc, OnceLock};

use lspk_plugin::ParamValues;
use reagent_engine::Registry;

/// Reagent's registries, made once: the parameter space, the modulation
/// sources and destinations.
fn registry() -> &'static Arc<Registry> {
    static REGISTRY: OnceLock<Arc<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Registry::new)
}

/// Reagent's parameter values, at their defaults, described by the space.
fn values(registry: &Registry) -> ParamValues {
    ParamValues::with_space(registry.space.descriptors(), Arc::new(registry.space.clone()))
        .expect("Reagent's parameter space")
}

/// The preset `bytes` hold, through the whole reader, and its round trip.
pub fn check_preset(bytes: &[u8]) {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let registry = registry();
    let values = values(registry);
    let Ok(preset) = reagent::preset::decode(text, &values, registry) else {
        return;
    };
    let state: reagent::preset::State =
        serde_json::from_value(preset.state.clone()).expect("a decoded state is Reagent's");
    // Apply it, as loading a file does, and save that patch again.
    lspk_presets::apply(&preset, &values);
    let saved = reagent::preset::encode(&values, &state).expect("a patch that loaded saves");
    let again = reagent::preset::decode(&saved, &values, registry)
        .expect("a preset that just saved loads again");
    assert_eq!(
        again, preset,
        "saving and loading a preset changed it: {saved}"
    );
}
