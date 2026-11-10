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

/// Build a sound from a function of time (seconds) and a noise source.
fn synth(seconds: f32, mut f: impl FnMut(f32, f32) -> f32) -> Vec<u8> {
    let mut rng = Rng::new(0xB111);
    let n = (seconds * RATE as f32) as usize;
    let mut lp = 0.0f32;
    let samples: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            // Low-passed noise sounds like felt and wood rather than hiss.
            lp += (rng.signed() as f32 - lp) * 0.35;
            let fade = ((n - i) as f32 / (RATE as f32 * 0.004)).min(1.0);
            f(t, lp) * fade
        })
        .collect();
    wav(&samples, 1)
}

fn tone(t: f32, hz: f32) -> f32 {
    (t * hz * std::f32::consts::TAU).sin()
}

pub struct Sounds {
    clack: Sound,
    cushion: Sound,
    pocket: Sound,
    cue: Sound,
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
        // Two phenolic balls meeting: a very short, bright knock.
        let clack = synth(0.06, |t, n| {
            let env = (-t * 150.0).exp();
            let body = tone(t, 2350.0) * 0.5 + tone(t, 3900.0) * 0.3 + tone(t, 5600.0) * 0.15;
            (body * env + n * (-t * 900.0).exp() * 0.9) * 0.9
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
        // Leather tip on the cue ball.
        let cue = synth(0.07, |t, n| {
            let env = (-t * 110.0).exp();
            (tone(t, 880.0) * 0.45 + tone(t, 1750.0) * 0.25) * env + n * (-t * 500.0).exp() * 0.8
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
            clack: load_sound_from_bytes(&clack).await.ok()?,
            cushion: load_sound_from_bytes(&cushion).await.ok()?,
            pocket: load_sound_from_bytes(&pocket).await.ok()?,
            cue: load_sound_from_bytes(&cue).await.ok()?,
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
        self.play(&self.clack, ((speed / 4.0).powf(0.7)) as f32);
    }
    pub fn cushion_hit(&self, speed: f64) {
        self.play(&self.cushion, ((speed / 4.0).powf(0.8)) as f32);
    }
    pub fn pocket(&self) {
        self.play(&self.pocket, 0.9);
    }
    pub fn cue_strike(&self, speed: f64) {
        self.play(&self.cue, (0.25 + speed / 9.0) as f32);
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
