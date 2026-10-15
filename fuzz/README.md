# Fuzz targets

Fuzz targets for Reagent's file readers, run with [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) and libFuzzer. This folder is its own Cargo workspace with its own `Cargo.lock`, outside the product's workspace, because cargo-fuzz needs a nightly toolchain and the product builds on the pinned stable one.

| Target | What it feeds | Checks |
|---|---|---|
| `preset` | Bytes as read from a `.reagent` preset file: the plug-in kit's format (JSON, schema version, migrations, symbolic enums), then Reagent's own state (the modulation matrix, each oscillator slot's engines) | No panic. Anything the reader accepts saves again and loads back as the same patch. |

## Run it

You need a nightly toolchain and cargo-fuzz once:

```sh
rustup toolchain install nightly --profile minimal
cargo install --locked cargo-fuzz
```

Then, from this folder, seed the target with a preset or two and run it:

```sh
cd fuzz
cargo +nightly fuzz run preset corpus/preset -- -max_total_time=300
```

The first folder after the target is the working corpus, which libFuzzer grows; `../crates/plugin/tests` holds no fixtures, since the tests write their preset files themselves, so seeding is optional. Corpora, crash artifacts, and build output stay out of Git (`.gitignore`). A crash writes its input to `artifacts/<target>/`; turn it into a regression test in `crates/plugin` (its `presets` tests) before fixing it.
