//! The MTS-ESP client: the project's microtuning master, read through its
//! shared library.
//!
//! ODDSound's library (`libMTS`, installed with any MTS-ESP master) holds one
//! table of 128 frequencies, which the master rewrites, and answers whether a
//! master is connected and whether a key should play. A client registers with
//! the library, then reads the table. This follows the protocol of
//! ODDSound's published client code (ISC licence), for the global table only:
//! the multi-channel tables aren't read, as that code doesn't read them for a
//! client that doesn't know a note's channel.

use std::ffi::{c_char, c_schar};

use libloading::Library;

use super::Master;

/// Where the library is installed, per platform.
#[cfg(target_os = "macos")]
fn paths() -> Vec<std::path::PathBuf> {
    vec!["/Library/Application Support/MTS-ESP/libMTS.dylib".into()]
}

#[cfg(target_os = "windows")]
fn paths() -> Vec<std::path::PathBuf> {
    std::env::var_os("CommonProgramFiles")
        .map(|dir| {
            std::path::Path::new(&dir)
                .join("MTS-ESP")
                .join("LIBMTS.dll")
        })
        .into_iter()
        .collect()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn paths() -> Vec<std::path::PathBuf> {
    vec!["/usr/local/lib/libMTS.so".into()]
}

/// A registered client of the MTS-ESP library.
pub struct Mts {
    has_master: unsafe extern "C" fn() -> bool,
    should_filter: Option<unsafe extern "C" fn(c_char, c_schar) -> bool>,
    deregister: Option<unsafe extern "C" fn()>,
    /// The library's table of 128 frequencies, valid while it is loaded.
    table: *const f64,
    /// Keeps the library loaded.
    _library: Library,
}

// SAFETY: the library's functions are callable from any thread, and its
// table is plain data the master rewrites; the pointer is read only.
unsafe impl Send for Mts {}
// SAFETY: as for `Send`.
unsafe impl Sync for Mts {}

impl Mts {
    /// Registers with the library, if it is installed and answers as an
    /// MTS-ESP library. Loads a file, so never call it on the audio thread.
    #[must_use]
    pub fn connect() -> Option<Self> {
        paths().iter().find_map(|path| {
            // SAFETY: loading ODDSound's library runs its initialisers, as
            // every MTS-ESP client does.
            let library = unsafe { Library::new(path) }.ok()?;
            // SAFETY: each symbol is looked up by the name and signature
            // ODDSound's client header gives it, and is called only through
            // that signature while `_library` keeps it loaded.
            unsafe {
                let register = library
                    .get::<unsafe extern "C" fn()>(b"MTS_RegisterClient\0")
                    .ok()
                    .map(|f| *f);
                let get_table = *library
                    .get::<unsafe extern "C" fn() -> *const f64>(b"MTS_GetTuningTable\0")
                    .ok()?;
                let has_master = *library
                    .get::<unsafe extern "C" fn() -> bool>(b"MTS_HasMaster\0")
                    .ok()?;
                let should_filter = library
                    .get::<unsafe extern "C" fn(c_char, c_schar) -> bool>(b"MTS_ShouldFilterNote\0")
                    .ok()
                    .map(|f| *f);
                let deregister = library
                    .get::<unsafe extern "C" fn()>(b"MTS_DeregisterClient\0")
                    .ok()
                    .map(|f| *f);
                let table = get_table();
                if table.is_null() {
                    return None;
                }
                if let Some(register) = register {
                    register();
                }
                Some(Self {
                    has_master,
                    should_filter,
                    deregister,
                    table,
                    _library: library,
                })
            }
        })
    }
}

impl Master for Mts {
    fn online(&self) -> bool {
        // SAFETY: a function of the loaded library, with no arguments.
        unsafe { (self.has_master)() }
    }

    fn frequency(&self, key: u8) -> f64 {
        // SAFETY: the table holds 128 frequencies for as long as the library
        // is loaded, and the index is below 128. The master may rewrite it,
        // so the read is volatile.
        unsafe { self.table.add(usize::from(key & 127)).read_volatile() }
    }

    fn filtered(&self, key: u8) -> bool {
        // SAFETY: a function of the loaded library; -1 says the note's
        // channel isn't known.
        self.should_filter
            .is_some_and(|f| unsafe { f((key & 127) as c_char, -1) })
    }
}

impl Drop for Mts {
    fn drop(&mut self) {
        if let Some(deregister) = self.deregister {
            // SAFETY: a function of the library, still loaded here.
            unsafe { deregister() };
        }
    }
}
