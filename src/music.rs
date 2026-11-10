//! Background music: a 16-bar lounge-jazz loop (electric piano, vibraphone,
//! upright bass and brushed drums) written and rendered here in code, so the
//! game needs no audio files and no music licence. It takes a moment to
//! render, so the game does it on a worker thread at start-up.

use crate::physics::Rng;
use std::f32::consts::{FRAC_PI_4, TAU};

const RATE: usize = 44_100;
const BPM: f32 = 84.0;
const BARS: usize = 16;
/// Where a swung off-beat eighth lands inside the beat.
const SWING: f32 = 0.64;

struct Chord {
    /// Bass root (MIDI note).
    bass: i32,
    /// Intervals above the root the walking bass leans on.
    third: i32,
    fifth: i32,
    /// Electric piano voicing (MIDI notes).
    keys: &'static [i32],
}

/// A gentle tour around F major: I vi ii V, iii VI ii V, IV iv iii VI, ii V I.
const CHORDS: [Chord; BARS] = [
    Chord { bass: 41, third: 4, fifth: 7, keys: &[57, 60, 64, 67] }, // Fmaj9
    Chord { bass: 38, third: 3, fifth: 7, keys: &[53, 57, 60, 64] }, // Dm9
    Chord { bass: 43, third: 3, fifth: 7, keys: &[53, 57, 58, 62] }, // Gm9
    Chord { bass: 36, third: 4, fifth: 7, keys: &[52, 57, 58, 62] }, // C13
    Chord { bass: 45, third: 3, fifth: 7, keys: &[55, 60, 64, 67] }, // Am7
    Chord { bass: 38, third: 4, fifth: 7, keys: &[54, 60, 64, 69] }, // D9
    Chord { bass: 43, third: 3, fifth: 7, keys: &[53, 57, 58, 62] }, // Gm9
    Chord { bass: 36, third: 4, fifth: 7, keys: &[52, 57, 58, 64] }, // C13
    Chord { bass: 34, third: 4, fifth: 7, keys: &[57, 60, 62, 65] }, // Bbmaj9
    Chord { bass: 34, third: 3, fifth: 7, keys: &[55, 61, 65, 67] }, // Bbm6
    Chord { bass: 45, third: 3, fifth: 7, keys: &[55, 60, 64, 67] }, // Am7
    Chord { bass: 38, third: 4, fifth: 7, keys: &[54, 60, 64, 69] }, // D9
    Chord { bass: 43, third: 3, fifth: 7, keys: &[53, 57, 58, 62] }, // Gm9
    Chord { bass: 36, third: 4, fifth: 7, keys: &[52, 57, 58, 64] }, // C13
    Chord { bass: 41, third: 4, fifth: 7, keys: &[57, 60, 64, 67] }, // Fmaj9
    Chord { bass: 36, third: 5, fifth: 7, keys: &[58, 62, 65, 67] }, // C9sus4
];

/// The vibraphone tune: (bar, beat, MIDI note, length in beats). Every note
/// is a tone of the chord under it.
const MELODY: &[(usize, f32, i32, f32)] = &[
    (0, 1.0, 72, 0.6), (0, 1.0 + SWING, 69, 0.4), (0, 2.0, 67, 1.6),
    (1, 2.0 + SWING, 65, 0.4), (1, 3.0, 69, 1.0),
    (2, 0.0, 70, 1.6), (2, 2.0, 69, 0.6), (2, 2.0 + SWING, 65, 1.3),
    (3, 1.0, 67, 0.6), (3, 1.0 + SWING, 69, 0.4), (3, 2.0, 70, 0.6), (3, 2.0 + SWING, 74, 1.3),
    (4, 0.0, 72, 1.0), (4, 1.0, 76, 2.6),
    (5, 2.0, 74, 0.6), (5, 2.0 + SWING, 72, 1.3),
    (6, 0.0, 70, 0.6), (6, SWING, 69, 0.4), (6, 1.0, 65, 0.6), (6, 1.0 + SWING, 62, 2.0),
    (7, 2.0, 64, 0.6), (7, 2.0 + SWING, 67, 0.4), (7, 3.0, 70, 1.0),
    (8, 0.0, 69, 2.0), (8, 2.0 + SWING, 72, 0.4), (8, 3.0, 74, 1.0),
    (9, 0.0, 73, 2.0), (9, 2.0 + SWING, 70, 1.3),
    (10, 1.0, 67, 0.6), (10, 1.0 + SWING, 69, 1.8),
    (11, 1.0, 66, 0.6), (11, 1.0 + SWING, 69, 0.4), (11, 2.0, 72, 0.6), (11, 2.0 + SWING, 76, 1.3),
    (12, 0.0, 74, 1.0), (12, 1.0, 70, 0.6), (12, 1.0 + SWING, 69, 1.8),
    (13, 1.0, 67, 0.6), (13, 1.0 + SWING, 64, 0.4), (13, 2.0, 67, 0.6), (13, 2.0 + SWING, 70, 1.3),
    (14, 0.0, 69, 3.6),
    (15, 2.0 + SWING, 67, 1.2),
];

fn hz(midi: i32) -> f32 {
    440.0 * 2f32.powf((midi - 69) as f32 / 12.0)
}

/// A stereo loop. Notes that ring past the end wrap round to the start, so
/// the loop has no seam.
struct Mix {
    left: Vec<f32>,
    right: Vec<f32>,
}

impl Mix {
    fn len(&self) -> usize {
        self.left.len()
    }

    /// Add a voice starting at `start` seconds for `seconds`, panned from -1
    /// (left) to 1 (right). `voice` gets the time since the note started.
    fn add(&mut self, start: f32, seconds: f32, pan: f32, mut voice: impl FnMut(f32) -> f32) {
        let n = self.len();
        let first = (start * RATE as f32) as usize;
        let count = ((seconds * RATE as f32) as usize).min(n);
        let angle = (pan.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
        let (gl, gr) = (angle.cos(), angle.sin());
        for i in 0..count {
            let s = voice(i as f32 / RATE as f32);
            let at = (first + i) % n;
            self.left[at] += s * gl;
            self.right[at] += s * gr;
        }
    }
}

/// Electric piano: a soft FM tone whose bark fades into a round bell.
fn piano(t: f32, f: f32, hold: f32) -> f32 {
    let attack = (t / 0.004).min(1.0);
    let release = if t > hold { (-(t - hold) * 9.0).exp() } else { 1.0 };
    let index = 1.3 * (-5.0 * t).exp() + 0.2;
    let tone = (TAU * f * t + index * (TAU * f * t).sin()).sin();
    let bell = (TAU * f * 2.0 * t).sin() * (-6.0 * t).exp() * 0.12;
    (tone * (-1.2 * t).exp() + bell) * attack * release
}

/// Vibraphone: a pure bar tone with its high fourth partial and a slow
/// motor tremolo.
fn vibes(t: f32, f: f32, hold: f32) -> f32 {
    let attack = (t / 0.003).min(1.0);
    let release = if t > hold { (-(t - hold) * 5.0).exp() } else { 1.0 };
    let tremolo = 1.0 + 0.22 * (TAU * 5.2 * t).sin();
    let tone = (TAU * f * t).sin() + (TAU * f * 4.0 * t).sin() * 0.08 * (-9.0 * t).exp();
    tone * (-1.5 * t).exp() * attack * release * tremolo
}

/// Upright bass: a warm pluck whose upper harmonics die first.
fn bass(t: f32, f: f32, hold: f32) -> f32 {
    let attack = (t / 0.006).min(1.0);
    let release = if t > hold { (-(t - hold) * 30.0).exp() } else { 1.0 };
    let tone = (TAU * f * t).sin()
        + (TAU * f * 2.0 * t).sin() * 0.35 * (-6.0 * t).exp()
        + (TAU * f * 3.0 * t).sin() * 0.12 * (-10.0 * t).exp();
    tone * (-2.6 * t).exp() * attack * release
}

pub fn render() -> Vec<u8> {
    let beat = 60.0 / BPM;
    let bar = beat * 4.0;
    let frames = (bar * BARS as f32 * RATE as f32).round() as usize;
    let mut mix = Mix { left: vec![0.0; frames], right: vec![0.0; frames] };
    let mut rng = Rng::new(0x1A2B);

    for (b, chord) in CHORDS.iter().enumerate() {
        let t0 = b as f32 * bar;
        let next = &CHORDS[(b + 1) % BARS];

        // Piano comping: a few relaxed patterns, chords rolled slightly.
        let hits: &[(f32, f32)] = match b % 4 {
            0 => &[(0.0, 1.6), (2.0 + SWING, 1.2)],
            1 => &[(0.0, 0.9), (1.0 + SWING, 0.4), (3.0, 0.9)],
            2 => &[(0.0, 2.0), (3.0 + SWING, 0.8)],
            _ => &[(0.0, 1.0), (1.0 + SWING, 0.4), (2.0 + SWING, 1.0)],
        };
        for &(at, len) in hits {
            let velocity = 0.8 + rng.f64() as f32 * 0.2;
            for (k, &note) in chord.keys.iter().enumerate() {
                let f = hz(note);
                let hold = len * beat;
                mix.add(t0 + at * beat + k as f32 * 0.011, hold + 0.6, -0.3, |t| {
                    piano(t, f, hold) * 0.085 * velocity
                });
            }
        }

        // Walking bass: root, third, fifth, then a step into the next chord.
        let approach = next.bass + if b % 2 == 0 { -1 } else { 1 };
        let walk = [chord.bass, chord.bass + chord.third, chord.bass + chord.fifth, approach];
        for (k, &note) in walk.iter().enumerate() {
            let f = hz(note);
            let hold = beat * 0.92;
            let accent = if k == 0 { 1.0 } else { 0.85 };
            mix.add(t0 + k as f32 * beat, hold + 0.1, 0.0, |t| bass(t, f, hold) * 0.30 * accent);
        }

        // Drums, kept well in the background.
        for k in 0..4 {
            let at = t0 + k as f32 * beat;
            // Feathered kick on 1 and 3.
            if k % 2 == 0 {
                mix.add(at, 0.25, 0.0, |t| {
                    (TAU * (48.0 * t + 14.0 * (1.0 - (-t * 30.0).exp()) / 30.0)).sin() * (-t * 16.0).exp() * 0.18
                });
            }
            // Brush swish on 2 and 4.
            if k % 2 == 1 {
                let mut low = 0.0f32;
                let mut noise = Rng::new(rng.next_u64());
                mix.add(at - 0.03, 0.3, -0.15, |t| {
                    low += (noise.signed() as f32 - low) * 0.18;
                    low * (t / 0.04).min(1.0) * (-t * 10.0).exp() * 0.07
                });
            }
            // Ride cymbal: every beat, plus the swung skip note on 2 and 4.
            let mut taps = vec![(at, 0.030)];
            if k % 2 == 1 {
                taps.push((at + SWING * beat, 0.020));
            }
            for (tap, amp) in taps {
                let mut prev = 0.0f32;
                let mut noise = Rng::new(rng.next_u64());
                mix.add(tap, 0.5, 0.45, |t| {
                    let n = noise.signed() as f32;
                    let hiss = n - prev;
                    prev = n;
                    let ping = ((TAU * 5400.0 * t).sin() + (TAU * 7950.0 * t).sin()) * 0.18;
                    (hiss * (-t * 22.0).exp() + ping * (-t * 9.0).exp()) * amp
                });
            }
        }
    }

    for &(b, at, note, len) in MELODY {
        let f = hz(note);
        let hold = len * beat;
        mix.add(b as f32 * bar + at * beat, hold + 1.2, 0.35, |t| vibes(t, f, hold) * 0.13);
    }

    finish(mix)
}

/// Warm the top end, round off peaks and convert to a 16-bit stereo WAV.
fn finish(mut mix: Mix) -> Vec<u8> {
    let n = mix.len();
    for ch in [&mut mix.left, &mut mix.right] {
        // One-pole low-pass, run over the loop twice so its state at the
        // start matches the end and the loop point stays seamless.
        let mut state = 0.0f32;
        for pass in 0..2 {
            for s in ch.iter_mut() {
                state += (*s - state) * 0.55;
                if pass == 1 {
                    *s = state;
                }
            }
        }
    }
    let peak = mix
        .left
        .iter()
        .chain(mix.right.iter())
        .fold(0.0f32, |m, s| m.max(s.abs()))
        .max(1e-6);
    let gain = 0.85 / peak;
    let mut interleaved = Vec::with_capacity(n * 2);
    for i in 0..n {
        interleaved.push((mix.left[i] * gain * 1.3).tanh() / 1.3_f32.tanh());
        interleaved.push((mix.right[i] * gain * 1.3).tanh() / 1.3_f32.tanh());
    }
    crate::audio::wav(&interleaved, 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn music_is_a_full_seamless_loop() {
        let bytes = render();
        let data = &bytes[44..];
        let samples: Vec<i16> = data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        // About 46 seconds of stereo audio.
        let seconds = samples.len() as f32 / 2.0 / RATE as f32;
        assert!((seconds - 45.7).abs() < 0.1, "{seconds}");
        // Loud enough to hear, never clipping.
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!(peak > 20_000 && peak < 32_767, "{peak}");
        // No click where the loop wraps round.
        let (a, b) = (samples[samples.len() - 2] as i32, samples[0] as i32);
        assert!((a - b).abs() < 4_000, "{a} -> {b}");
    }
}
