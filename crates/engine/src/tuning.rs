//! Tuning: how far each key sounds from twelve-tone equal temperament.
//!
//! A tuning is a [`Table`] of 128 offsets in semitones from the equal
//! temperament that puts A4 at 440 Hz: a Scala scale with its keyboard
//! mapping (`.scl` and `.kbm`) or a `.tun` file, read into one. The engine
//! adds a note's offset to its pitch, and Reference pitch and Main tuning
//! then apply on top, as they do to any other note (engine-core R136, R137).
//!
//! Where a note's tuning comes from, in order: the file this instance
//! loaded; otherwise an MTS-ESP master, when the client is enabled and one
//! is connected; otherwise equal temperament. A file always takes
//! precedence over MTS-ESP, so an instance can be tuned differently from
//! the project's master. MTS-ESP is read at each note-on, or all through a
//! note when Note-on only is off.
//!
//! The plug-in's main thread writes a [`Control`]; the audio thread reads it
//! without locking. The MTS-ESP library sits behind [`Master`], which tests
//! stand in for; [`mts::Mts`] is the real one.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

pub mod mts;

/// The keys a tuning covers.
pub const KEYS: usize = 128;

/// MIDI note 69, A4.
const A4: f64 = 69.0;

/// The semitones by which a note at `hz` differs from key `key` in equal
/// temperament at A4 = 440 Hz.
fn offset_of(key: usize, hz: f64) -> f32 {
    (12.0 * (hz / 440.0).log2() - (key as f64 - A4)) as f32
}

/// A tuning: each key's offset from equal temperament, and which keys play
/// nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    semitones: [f32; KEYS],
    filtered: u128,
}

impl Table {
    /// Twelve-tone equal temperament: every key in place.
    #[must_use]
    pub const fn equal() -> Self {
        Self {
            semitones: [0.0; KEYS],
            filtered: 0,
        }
    }

    /// Key `key`'s offset from equal temperament, in semitones.
    #[must_use]
    pub fn semitones(&self, key: u8) -> f32 {
        self.semitones[usize::from(key) & 127]
    }

    /// Whether key `key` plays nothing.
    #[must_use]
    pub fn filtered(&self, key: u8) -> bool {
        self.filtered >> (key & 127) & 1 == 1
    }

    /// The table whose key `k` sounds at `hz(k)`, or plays nothing for `None`.
    fn from_frequencies(hz: impl Fn(usize) -> Option<f64>) -> Self {
        let mut table = Self::equal();
        for key in 0..KEYS {
            match hz(key) {
                Some(hz) => table.semitones[key] = offset_of(key, hz),
                None => table.filtered |= 1 << key,
            }
        }
        table
    }

    /// The table a Scala scale (`.scl`) tunes, mapped to the keys by a
    /// keyboard mapping (`.kbm`); with none, the scale starts at middle C
    /// (key 60) and A4 (key 69) sounds at 440 Hz. Keys the mapping leaves
    /// out play nothing.
    ///
    /// # Errors
    ///
    /// Either file doesn't read as its format.
    pub fn from_scala(scl: &str, kbm: Option<&str>) -> Result<Self, String> {
        let cents = parse_scl(scl)?;
        let map = match kbm {
            Some(text) => parse_kbm(text)?,
            None => KeyMap::default(),
        };
        let Some(reference) = map.cents(map.reference_note, &cents) else {
            return Err("the keyboard mapping leaves its own reference key unmapped".to_owned());
        };
        Ok(Self::from_frequencies(|key| {
            if key < map.first || key > map.last {
                return None;
            }
            let at = map.cents(key, &cents)?;
            Some(map.reference_hz * ((at - reference) / 1200.0).exp2())
        }))
    }

    /// The table an AnaMark `.tun` file holds: its `[Exact Tuning]` section
    /// if it has one, else `[Tuning]`. Each `note n=` line gives key `n`'s
    /// pitch in cents above MIDI note 0 in equal temperament (8.1758 Hz); a
    /// key the file doesn't name stays in equal temperament.
    ///
    /// # Errors
    ///
    /// The file has neither section, a line that isn't a number of cents, or
    /// holds a functional tuning, which isn't read.
    pub fn from_tun(text: &str) -> Result<Self, String> {
        let mut sections: Vec<(String, Vec<(usize, f64)>)> = Vec::new();
        for line in text.lines().map(str::trim) {
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                sections.push((name.trim().to_ascii_lowercase(), Vec::new()));
            } else if let Some((key, value)) = line.split_once('=')
                && let Some((_, notes)) = sections.last_mut()
            {
                let key = key.trim().to_ascii_lowercase();
                let Some(n) = key.strip_prefix("note") else {
                    continue;
                };
                let Ok(n) = n.trim().parse::<usize>() else {
                    continue;
                };
                let cents = value
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| format!("`{line}` isn't a number of cents"))?;
                if n < KEYS {
                    notes.push((n, cents));
                }
            }
        }
        if sections.iter().any(|(name, _)| name == "functional tuning") {
            return Err("functional tunings aren't supported".to_owned());
        }
        let found = |name: &str| sections.iter().find(|(n, _)| n == name);
        let Some((_, notes)) = found("exact tuning").or_else(|| found("tuning")) else {
            return Err("the file has no [Tuning] or [Exact Tuning] section".to_owned());
        };
        let mut cents: [f64; KEYS] = std::array::from_fn(|key| key as f64 * 100.0);
        for &(n, c) in notes {
            cents[n] = c;
        }
        let base = 440.0 * (-A4 / 12.0).exp2();
        Ok(Self::from_frequencies(|key| {
            Some(base * (cents[key] / 1200.0).exp2())
        }))
    }
}

/// A Scala scale's degrees above its first, in cents; the last is the
/// period (usually the octave).
fn parse_scl(text: &str) -> Result<Vec<f64>, String> {
    let mut lines = text.lines().filter(|l| !l.trim_start().starts_with('!'));
    // The first line describes the scale and may be empty.
    lines.next().ok_or("the scale file is empty")?;
    let count: usize = lines
        .next()
        .and_then(|l| l.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .ok_or("the scale file doesn't say how many notes it has")?;
    let mut cents = Vec::with_capacity(count);
    for line in lines.take(count) {
        let word = line.split_whitespace().next().unwrap_or("");
        let value = if word.contains('.') {
            word.parse::<f64>().ok()
        } else {
            let (num, den) = word.split_once('/').unwrap_or((word, "1"));
            match (num.parse::<f64>(), den.parse::<f64>()) {
                (Ok(n), Ok(d)) if n > 0.0 && d > 0.0 => Some(1200.0 * (n / d).log2()),
                _ => None,
            }
        };
        cents.push(value.ok_or_else(|| format!("`{line}` isn't a pitch"))?);
    }
    if cents.is_empty() {
        return Err("a scale has at least one note".to_owned());
    }
    if cents.len() != count {
        return Err(format!(
            "the scale says it has {count} notes and lists {}",
            cents.len()
        ));
    }
    Ok(cents)
}

/// A Scala keyboard mapping.
struct KeyMap {
    /// The keys it covers.
    first: usize,
    last: usize,
    /// The key the scale's first degree sits on.
    middle: usize,
    reference_note: usize,
    reference_hz: f64,
    /// The scale degree that is an octave up the mapping; `None` is the
    /// scale's length.
    formal_octave: Option<i64>,
    /// Which degree each key of one repeat plays, `None` for a key left out;
    /// empty for a linear mapping.
    degrees: Vec<Option<i64>>,
}

impl Default for KeyMap {
    fn default() -> Self {
        Self {
            first: 0,
            last: KEYS - 1,
            middle: 60,
            reference_note: 69,
            reference_hz: 440.0,
            formal_octave: None,
            degrees: Vec::new(),
        }
    }
}

impl KeyMap {
    /// Key `key`'s pitch in cents above the scale's first degree, `None` if
    /// the mapping leaves it out.
    fn cents(&self, key: usize, scale: &[f64]) -> Option<f64> {
        let delta = key as i64 - self.middle as i64;
        let len = scale.len() as i64;
        let degree = if self.degrees.is_empty() {
            delta
        } else {
            let size = self.degrees.len() as i64;
            let repeats = delta.div_euclid(size);
            let formal = self.formal_octave.unwrap_or(len);
            self.degrees[delta.rem_euclid(size) as usize]? + repeats * formal
        };
        let period = scale[scale.len() - 1];
        let octaves = degree.div_euclid(len);
        let within = degree.rem_euclid(len);
        let above = if within == 0 {
            0.0
        } else {
            scale[within as usize - 1]
        };
        Some(octaves as f64 * period + above)
    }
}

fn parse_kbm(text: &str) -> Result<KeyMap, String> {
    let mut words = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('!'))
        .filter_map(|l| l.split_whitespace().next());
    let mut number = |what: &str| {
        words
            .next()
            .and_then(|w| w.parse::<f64>().ok())
            .ok_or_else(|| format!("the keyboard mapping has no {what}"))
    };
    let size = number("size")? as usize;
    let first = number("first key")? as usize;
    let last = number("last key")? as usize;
    let middle = number("middle key")? as usize;
    let reference_note = number("reference key")? as usize;
    let reference_hz = number("reference frequency")?;
    let formal = number("formal octave")? as i64;
    if reference_hz <= 0.0 || middle >= KEYS || reference_note >= KEYS {
        return Err("the keyboard mapping names a key or a frequency out of range".to_owned());
    }
    let mut degrees = Vec::with_capacity(size);
    for _ in 0..size {
        let word = words
            .next()
            .ok_or("the keyboard mapping is short of keys")?;
        degrees.push(match word {
            "x" | "X" => None,
            n => Some(
                n.parse::<i64>()
                    .map_err(|_| format!("`{n}` isn't a scale degree"))?,
            ),
        });
    }
    Ok(KeyMap {
        first,
        last: last.min(KEYS - 1),
        middle,
        reference_note,
        reference_hz,
        formal_octave: (formal != 0).then_some(formal),
        degrees,
    })
}

/// A source of tuning outside the instance: an MTS-ESP master. Read on the
/// audio thread, so it takes no lock.
pub trait Master: Send + Sync {
    /// Whether a master is connected.
    fn online(&self) -> bool;
    /// Key `key`'s frequency, in hertz.
    fn frequency(&self, key: u8) -> f64;
    /// Whether the master wants key `key` to play nothing.
    fn filtered(&self, key: u8) -> bool;
}

/// What the plug-in's main thread sets and the audio thread reads: the
/// loaded file's table, and how MTS-ESP is used.
pub struct Control {
    semitones: [AtomicU32; KEYS],
    filtered: [AtomicU64; 2],
    file: AtomicBool,
    mts: AtomicBool,
    note_on_only: AtomicBool,
}

impl Control {
    /// A control with no file, and MTS-ESP on and read all through a note
    /// (the MTS-ESP client's recommendation).
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            semitones: std::array::from_fn(|_| AtomicU32::new(0)),
            filtered: [AtomicU64::new(0), AtomicU64::new(0)],
            file: AtomicBool::new(false),
            mts: AtomicBool::new(true),
            note_on_only: AtomicBool::new(false),
        })
    }

    /// Tunes this instance by `table`, or by nothing of its own for `None`.
    /// A note already sounding keeps the tuning it started with.
    pub fn set_file(&self, table: Option<&Table>) {
        if let Some(table) = table {
            for (cell, s) in self.semitones.iter().zip(&table.semitones) {
                cell.store(s.to_bits(), Ordering::Relaxed);
            }
            self.filtered[0].store(table.filtered as u64, Ordering::Relaxed);
            self.filtered[1].store((table.filtered >> 64) as u64, Ordering::Relaxed);
        }
        self.file.store(table.is_some(), Ordering::Release);
    }

    /// Whether the MTS-ESP client is on, and whether it reads the master at
    /// a note's start only.
    pub fn set_mts(&self, enabled: bool, note_on_only: bool) {
        self.mts.store(enabled, Ordering::Relaxed);
        self.note_on_only.store(note_on_only, Ordering::Relaxed);
    }
}

/// The audio thread's side: the control, and the master if the plug-in has
/// one.
#[derive(Clone)]
pub struct Tuner {
    control: Arc<Control>,
    master: Option<Arc<dyn Master>>,
}

impl Tuner {
    /// A tuner over `control` and `master`.
    #[must_use]
    pub fn new(control: Arc<Control>, master: Option<Arc<dyn Master>>) -> Self {
        Self { control, master }
    }

    /// The master, when it is the source in use.
    fn master(&self) -> Option<&dyn Master> {
        let c = &self.control;
        if c.file.load(Ordering::Acquire) || !c.mts.load(Ordering::Relaxed) {
            return None;
        }
        self.master.as_deref().filter(|m| m.online())
    }

    /// Key `key`'s offset from equal temperament in semitones, or `None`
    /// when it plays nothing.
    #[must_use]
    pub fn offset(&self, key: u8) -> Option<f32> {
        let c = &self.control;
        if c.file.load(Ordering::Acquire) {
            let k = usize::from(key & 127);
            if c.filtered[k / 64].load(Ordering::Relaxed) >> (k % 64) & 1 == 1 {
                return None;
            }
            return Some(f32::from_bits(c.semitones[k].load(Ordering::Relaxed)));
        }
        match self.master() {
            Some(m) if m.filtered(key) => None,
            Some(m) => Some(offset_of(usize::from(key & 127), m.frequency(key))),
            None => Some(0.0),
        }
    }

    /// Whether a sounding note's offset can change under it: an MTS-ESP
    /// master is in use and Note-on only is off.
    #[must_use]
    pub fn follows_master(&self) -> bool {
        self.master().is_some() && !self.control.note_on_only.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWELVE: &str = "! 12.scl\nTwelve-tone equal\n12\n100.0\n200.\n300.\n400.\n500.\n600.\n700.\n800.\n900.\n1000.\n1100.\n2/1\n";

    // The frequency key `key` sounds at under `table`.
    fn hz(table: &Table, key: u8) -> f64 {
        440.0 * ((f64::from(key) - A4 + f64::from(table.semitones(key))) / 12.0).exp2()
    }

    #[test]
    fn a_scale_in_twelve_equal_with_no_mapping_leaves_the_keys_in_place() {
        let table = Table::from_scala(TWELVE, None).unwrap();
        for key in 0..128 {
            assert!(table.semitones(key).abs() < 1e-4, "key {key}");
        }
    }

    #[test]
    fn a_scale_repeats_at_its_period_from_the_middle_key() {
        // Five notes to the octave, one key each, the first on key 60 at
        // 261.625565 Hz.
        let scl = "Pentatonic\n5\n9/8\n5/4\n3/2\n5/3\n2/1\n";
        let kbm = "5\n0\n127\n60\n60\n261.625565\n5\n0\n1\n2\n3\n4\n";
        let table = Table::from_scala(scl, Some(kbm)).unwrap();
        let tonic = 261.625565;
        for (key, ratio) in [
            (60, 1.0),
            (61, 9.0 / 8.0),
            (62, 1.25),
            (64, 5.0 / 3.0),
            (65, 2.0),
            (66, 2.25),
            (59, 5.0 / 6.0),
            (55, 0.5),
            (70, 4.0),
        ] {
            assert!((hz(&table, key) / tonic - ratio).abs() < 1e-4, "key {key}");
        }
    }

    #[test]
    fn a_mapping_leaves_out_the_keys_it_marks_or_does_not_cover() {
        let kbm = "3\n40\n80\n60\n60\n261.625565\n12\n0\nx\n7\n";
        let table = Table::from_scala(TWELVE, Some(kbm)).unwrap();
        assert!(!table.filtered(60));
        assert!(table.filtered(61));
        assert!(!table.filtered(62));
        assert!(table.filtered(39));
        assert!(table.filtered(81));
    }

    #[test]
    fn a_tun_file_names_each_key_in_cents_above_note_zero() {
        // Key 69 a quarter tone sharp; the rest as written.
        let tun = "[Tuning]\nnote 0=0\nnote 69=6950\nnote 70=7000\n";
        let table = Table::from_tun(tun).unwrap();
        assert!((table.semitones(69) - 0.5).abs() < 1e-4);
        assert!(table.semitones(70).abs() < 1e-4);
        assert!(table.semitones(100).abs() < 1e-4);
    }

    #[test]
    fn files_that_are_not_tunings_are_refused() {
        assert!(Table::from_scala("", None).is_err());
        assert!(Table::from_scala("x\n2\n100.0\n", None).is_err());
        assert!(Table::from_tun("note 1=100").is_err());
        assert!(Table::from_tun("[Functional Tuning]\nBaseFreq=440").is_err());
    }
}
