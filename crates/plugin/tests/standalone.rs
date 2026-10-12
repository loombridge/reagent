//! Reagent as a standalone app, on a fake audio device: a MIDI note plays
//! at its pitch, and the audio loop stays off the heap.

use lspk_standalone::fake::FakeBackend;
use lspk_standalone::settings::Settings;
use lspk_standalone::{Standalone, Status};
use reagent::Reagent;

lspk_rt::install_trap!();

#[test]
fn a_note_from_midi_plays_through_the_device() {
    let fake = FakeBackend::new(vec![FakeBackend::device("speakers", true)]);
    let mut app = Standalone::<Reagent>::new(Box::new(fake.clone()), Settings::default(), None)
        .expect("the app");
    assert!(Standalone::<Reagent>::takes_notes() && !Standalone::<Reagent>::has_input());
    assert!(app.has_editor());
    app.start();
    assert!(matches!(app.status(), Status::Running(info) if info.sample_rate == 48_000));

    let silence = fake.render(512).expect("a buffer");
    assert!(silence.iter().all(|s| *s == 0.0));

    // A4 for a tenth of a second: 44 cycles of the Init patch's saw.
    app.midi().send([0x90, 69, 127]);
    let input = vec![0.0f32; 4800 * 2];
    let mut output = vec![0.0f32; 4800 * 2];
    assert!(lspk_rt::rt_test!({ fake.render_into(&input, &mut output) }));
    let left: Vec<f32> = output.iter().step_by(2).copied().collect();
    let peak = left.iter().fold(0.0f32, |p, s| p.max(s.abs()));
    assert!(peak > 0.05 && peak < 0.5, "peak {peak}");
    let pitch = lspk_test::measure::pitch(&left[1200..], 48_000.0).expect("a pitch");
    assert!((pitch - 440.0).abs() < 0.5, "{pitch} Hz");
}
