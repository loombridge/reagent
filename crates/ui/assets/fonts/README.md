# Bundled typefaces

Reagent draws with a typeface it brings, because the kit bundles none and the platform's own differs from one system to the next. `deny.toml` allows OFL-1.1 for fonts only. Each look in `src/look.rs` names one of these three; the look the maintainer chooses keeps its typeface and the other two are removed.

| File | Typeface | Licence | Where it came from |
|---|---|---|---|
| `barlow/Barlow-Regular.ttf` | Barlow Regular, by the Barlow Project Authors | SIL Open Font License 1.1, in `barlow/OFL.txt` | Google Fonts' `ofl/barlow`, unchanged |
| `ibm-plex-sans/IBMPlexSans-Regular.ttf` | IBM Plex Sans Regular, by IBM Corp. | SIL Open Font License 1.1, in `ibm-plex-sans/OFL.txt` | The `IBM/plex` repository's `plex-sans` TrueType files, unchanged |
| `atkinson-hyperlegible/AtkinsonHyperlegible-Regular.ttf` | Atkinson Hyperlegible Regular, by the Braille Institute of America | SIL Open Font License 1.1, in `atkinson-hyperlegible/OFL.txt` | Google Fonts' `ofl/atkinsonhyperlegible`, unchanged |

The font files aren't modified and aren't sold on their own, as the OFL requires; each licence sits beside its font and travels with the source.
