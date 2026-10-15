//! Fuzzes Reagent's preset reader: bytes as read from a `.reagent` file,
//! through the plug-in kit's format (JSON, schema version, migrations,
//! symbolic enums), then Reagent's own state in it (the modulation matrix,
//! each oscillator slot's engines).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    reagent_fuzz::check_preset(data);
});
