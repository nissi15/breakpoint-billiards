//! Sound effects and music, synthesised at start-up so the game ships as a
//! single file.

use crate::physics::Rng;
use macroquad::audio::{PlaySoundParams, Sound, load_sound_from_bytes, play_sound, set_sound_volume};
use std::sync::mpsc::{Receiver, channel};

/// Music volume for each step of the menu's MUSIC control (OFF, LOW, MID, HIGH).
pub const MUSIC_LEVELS: [f32; 4] = [0.0, 0.22, 0.40, 0.62];

const RATE: u32 = 44_100;

/// 16-bit PCM WAV; `samples` is interleaved when `channels` is 2.
pub fn wav(samples: &[f32], channels: u16) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2 * channels as u32).to_le_bytes());
    out.extend_from_slice(&(2 * channels).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// Build a sound from a function of time (seconds), soft low-passed noise
/// and raw white noise.
fn render(seconds: f32, mut f: impl FnMut(f32, f32, f32) -> f32) -> Vec<f32> {
    let mut rng = Rng::new(0xB111);
    let n = (seconds * RATE as f32) as usize;
    let mut lp = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let white = rng.signed() as f32;
            // Low-passed noise sounds like felt and wood rather than hiss.
            lp += (white - lp) * 0.35;
            let fade = ((n - i) as f32 / (RATE as f32 * 0.004)).min(1.0);
            f(t, lp, white) * fade
        })
        .collect()
}

fn synth(seconds: f32, mut f: impl FnMut(f32, f32) -> f32) -> Vec<u8> {
    wav(&render(seconds, |t, lp, _| f(t, lp)), 1)
}

/// Scale a sound so its loudest sample sits just under full level.
fn level(mut s: Vec<f32>) -> Vec<f32> {
    let peak = s.iter().fold(0.0f32, |m, x| m.max(x.abs())).max(1e-6);
    for x in &mut s {
        *x *= 0.92 / peak;
    }
    s
}

/// Two balls meeting, for one of three strengths (0 soft, 1 medium, 2 hard).
///
/// Measured ball-to-ball contact lasts only about 0.25 to 0.3 thousandths of
/// a second, and the balls themselves ring far above hearing, so the real
/// sound is not a note: it is one sharp broadband click centred around
/// 3-4 kHz that is over in a millisecond or two, followed by the table and
/// the room answering. A harder hit has a shorter contact, so it is brighter
/// as well as louder, and it shakes the slate bed enough to add a low thump.
fn clack_samples(strength: usize) -> Vec<f32> {
    let (hz, bed) = [(2500.0, 0.03), (3300.0, 0.07), (4200.0, 0.13)][strength.min(2)];
    level(render(0.07, |t, lp, white| {
        // The click itself: gone within a couple of milliseconds.
        let click = (tone(t, hz) * 0.75 + tone(t, hz * 1.62) * 0.35) * (-t * 1600.0).exp()
            + white * 0.55 * (-t * 2600.0).exp();
        // The cloth-covered slate and the room: a short dull tail.
        let tail = tone(t, hz * 0.52) * 0.16 * (-t * 380.0).exp() + lp * 0.10 * (-t * 110.0).exp();
        let thump = tone(t, 205.0) * bed * (-t * 55.0).exp();
        click + tail + thump
    }))
}

/// The cue tip on the cue ball (0 an ordinary stroke, 1 a hard one).
///
/// A leather tip stays on the ball four to eight times longer than two
/// balls touch (about 1 to 2 thousandths of a second; less for a hard,
/// fast hit), so this is a lower, rounder "tock" than the ball click, with
/// a little of the wooden shaft ringing after it.
fn cue_samples(strength: usize) -> Vec<f32> {
    let (hz, crack, shaft) = [(420.0, 0.30, 0.16), (640.0, 0.60, 0.26)][strength.min(1)];
    level(render(0.08, |t, lp, white| {
        let thud = tone(t, hz) * 0.75 * (-t * 420.0).exp();
        let leather = lp * 0.55 * (-t * 700.0).exp() + white * crack * 0.5 * (-t * 1900.0).exp();
        let wood = (tone(t, 1180.0) * shaft + tone(t, 2450.0) * shaft * 0.5) * (-t * 190.0).exp();
        thud + leather + wood
    }))
}

fn tone(t: f32, hz: f32) -> f32 {
    (t * hz * std::f32::consts::TAU).sin()
}

pub struct Sounds {
    /// Ball on ball: soft, medium and hard.
    clack: [Sound; 3],
    cushion: Sound,
    pocket: Sound,
    /// Tip on the cue ball: an ordinary stroke and a hard one.
    cue: [Sound; 2],
    click: Sound,
    chime: Sound,
    knock: Sound,
    rack: Sound,
    foul: Sound,
    pub enabled: bool,
    /// Background music, once the worker thread has rendered it.
    music: Option<Sound>,
    music_job: Option<Receiver<Vec<u8>>>,
    /// Index into MUSIC_LEVELS.
    pub music_level: u8,
    music_volume: f32,
    /// Silences everything without changing the player's setting
    /// (used for automated screenshots and self-play).
    pub muted: bool,
}

impl Sounds {
    pub async fn load() -> Option<Sounds> {
        // The music takes a moment to render; do it while the game starts.
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::music::render());
        });
        // Rubber cushion: a dull thud.
        let cushion = synth(0.12, |t, n| {
            let env = (-t * 55.0).exp();
            (tone(t, 150.0 - t * 300.0) * 0.8 + n * 0.5 * (-t * 120.0).exp()) * env
        });
        // Ball dropping into the pocket and rattling down.
        let pocket = synth(0.5, |t, n| {
            let mut s = 0.0;
            for (start, hz, amp) in [
                (0.0, 190.0, 1.0),
                (0.09, 240.0, 0.7),
                (0.17, 170.0, 0.5),
                (0.27, 210.0, 0.3),
                (0.36, 160.0, 0.2),
            ] {
                if t >= start {
                    let u = t - start;
                    s += (tone(u, hz) * 0.7 + n * 0.6) * (-u * 60.0).exp() * amp;
                }
            }
            s * 0.8
        });
        let click = synth(0.03, |t, _| tone(t, 1250.0) * (-t * 220.0).exp() * 0.5);
        let chime = synth(0.9, |t, _| {
            let mut s = 0.0;
            for (start, hz) in [
                (0.0, 523.25),
                (0.12, 659.25),
                (0.24, 783.99),
                (0.36, 1046.5),
            ] {
                if t >= start {
                    let u = t - start;
                    s += (tone(u, hz) + tone(u, hz * 2.0) * 0.25) * (-u * 6.0).exp() * 0.22;
                }
            }
            s
        });
        // Knuckles on the wooden rail: the player's way of saying "good shot".
        let knock = synth(0.10, |t, n| {
            (tone(t, 310.0) * 0.6 + tone(t, 620.0) * 0.25 + n * 0.5 * (-t * 200.0).exp())
                * (-t * 60.0).exp()
        });
        // Balls settling into the rack.
        let rack = synth(0.55, |t, n| {
            let mut s = 0.0;
            for k in 0..9 {
                let start = k as f32 * 0.045 + ((k * 7) % 5) as f32 * 0.006;
                if t >= start {
                    let u = t - start;
                    s += (tone(u, 2100.0 + 260.0 * (k % 4) as f32) * 0.35
                        + n * 0.5 * (-u * 800.0).exp())
                        * (-u * 130.0).exp();
                }
            }
            s * 0.6
        });
        // Referee's foul signal: two soft low notes.
        let foul = synth(0.42, |t, _| {
            let a = tone(t, 196.0) * (-t * 9.0).exp();
            let b = if t > 0.16 {
                tone(t - 0.16, 147.0) * (-(t - 0.16) * 9.0).exp()
            } else {
                0.0
            };
            (a + b) * 0.30
        });
        Some(Sounds {
            clack: [
                load_sound_from_bytes(&wav(&clack_samples(0), 1)).await.ok()?,
                load_sound_from_bytes(&wav(&clack_samples(1), 1)).await.ok()?,
                load_sound_from_bytes(&wav(&clack_samples(2), 1)).await.ok()?,
            ],
            cushion: load_sound_from_bytes(&cushion).await.ok()?,
            pocket: load_sound_from_bytes(&pocket).await.ok()?,
            cue: [
                load_sound_from_bytes(&wav(&cue_samples(0), 1)).await.ok()?,
                load_sound_from_bytes(&wav(&cue_samples(1), 1)).await.ok()?,
            ],
            click: load_sound_from_bytes(&click).await.ok()?,
            chime: load_sound_from_bytes(&chime).await.ok()?,
            knock: load_sound_from_bytes(&knock).await.ok()?,
            rack: load_sound_from_bytes(&rack).await.ok()?,
            foul: load_sound_from_bytes(&foul).await.ok()?,
            enabled: true,
            music: None,
            music_job: Some(rx),
            music_level: 2,
            music_volume: 0.0,
            muted: false,
        })
    }

    /// Start the music when it is ready and ease its volume toward the
    /// chosen level; it sits a little lower during a match so the balls
    /// stay clear.
    pub async fn update_music(&mut self, in_match: bool, dt: f32) {
        if self.music.is_none() {
            let Some(job) = &self.music_job else { return };
            let Ok(bytes) = job.try_recv() else { return };
            self.music_job = None;
            let Ok(sound) = load_sound_from_bytes(&bytes).await else { return };
            play_sound(&sound, PlaySoundParams { looped: true, volume: 0.0 });
            self.music = Some(sound);
        }
        let level = MUSIC_LEVELS[(self.music_level as usize).min(MUSIC_LEVELS.len() - 1)];
        let target = if self.muted { 0.0 } else { level * if in_match { 0.75 } else { 1.0 } };
        let before = self.music_volume;
        self.music_volume += (target - before) * (dt * 1.8).min(1.0);
        if (target - self.music_volume).abs() < 0.002 {
            self.music_volume = target;
        }
        if self.music_volume != before {
            if let Some(m) = &self.music {
                set_sound_volume(m, self.music_volume);
            }
        }
    }

    fn play(&self, s: &Sound, volume: f32) {
        if self.enabled && !self.muted && volume > 0.01 {
            play_sound(
                s,
                PlaySoundParams {
                    looped: false,
                    volume: volume.clamp(0.0, 1.0),
                },
            );
        }
    }

    /// `speed` is the closing speed of the two balls in m/s.
    pub fn ball_hit(&self, speed: f64) {
        let kind = if speed < 0.9 { 0 } else if speed < 3.2 { 1 } else { 2 };
        self.play(&self.clack[kind], ((speed / 4.0).powf(0.7)) as f32);
    }
    pub fn cushion_hit(&self, speed: f64) {
        self.play(&self.cushion, ((speed / 4.0).powf(0.8)) as f32);
    }
    pub fn pocket(&self) {
        self.play(&self.pocket, 0.9);
    }
    pub fn cue_strike(&self, speed: f64) {
        self.play(&self.cue[(speed > 4.5) as usize], (0.25 + speed / 9.0) as f32);
    }
    pub fn click(&self) {
        self.play(&self.click, 0.5);
    }
    pub fn knock(&self) {
        self.play(&self.knock, 0.85);
    }
    pub fn rack(&self) {
        self.play(&self.rack, 0.6);
    }
    pub fn foul(&self) {
        self.play(&self.foul, 0.7);
    }
    pub fn chime(&self) {
        self.play(&self.chime, 0.8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seconds until 90% of the sound's energy has been heard.
    fn length(s: &[f32]) -> f32 {
        let total: f32 = s.iter().map(|x| x * x).sum();
        let mut sum = 0.0;
        for (i, x) in s.iter().enumerate() {
            sum += x * x;
            if sum >= total * 0.9 {
                return i as f32 / RATE as f32;
            }
        }
        0.0
    }

    /// How bright the first two milliseconds are: the energy of the
    /// sample-to-sample change against the energy of the sound itself.
    fn brightness(s: &[f32]) -> f32 {
        let n = (RATE as f32 * 0.002) as usize;
        let change: f32 = s[..n].windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
        let energy: f32 = s[..n].iter().map(|x| x * x).sum();
        change / energy
    }

    #[test]
    fn ball_clicks_are_short_and_harder_hits_are_brighter() {
        let hits: Vec<Vec<f32>> = (0..3).map(clack_samples).collect();
        for h in &hits {
            assert!(h.iter().all(|x| x.abs() <= 1.0), "clipped");
            // A click, not a ring: nearly all of it inside a few milliseconds.
            // (the hardest hit also shakes the table, which lingers a little).
            assert!(length(h) < 0.015, "too long: {}", length(h));
            println!("click: {:.4} s, brightness {:.3}", length(h), brightness(h));
        }
        assert!(brightness(&hits[0]) < brightness(&hits[1]));
        assert!(brightness(&hits[1]) < brightness(&hits[2]));
    }

    #[test]
    fn the_cue_tip_is_lower_and_rounder_than_a_ball_click() {
        for k in 0..2 {
            let c = cue_samples(k);
            assert!(c.iter().all(|x| x.abs() <= 1.0), "clipped");
            assert!(brightness(&c) < brightness(&clack_samples(0)));
            println!("cue: {:.4} s, brightness {:.3}", length(&c), brightness(&c));
            assert!(length(&c) > 0.002 && length(&c) < 0.02, "length {}", length(&c));
        }
    }
}
