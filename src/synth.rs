//! A tiny voice synthesizer: oscillator + optional noise + envelope + one-pole
//! low-pass, with pitch slide and stereo pan. Everything is procedural, so the
//! built-in packs carry no copyrighted audio.

#[derive(Debug, Clone, Copy)]
pub enum Wave {
    /// Pulse wave with the given duty cycle (0.5 = square).
    Pulse(f32),
    Triangle,
    Sine,
}

#[derive(Debug, Clone, Copy)]
pub struct VoiceSpec {
    pub wave: Wave,
    pub freq: f32,
    /// Frequency at the end of the voice (pitch slide); equal to `freq` for none.
    pub freq_end: f32,
    /// Seconds.
    pub attack: f32,
    /// Exponential decay time constant, seconds.
    pub decay: f32,
    /// Hard stop, seconds.
    pub dur: f32,
    pub gain: f32,
    /// 0 = pure tone, 1 = pure noise.
    pub noise: f32,
    /// Low-pass cutoff in Hz (0 = off).
    pub lowpass: f32,
    /// -1 (left) .. 1 (right).
    pub pan: f32,
    /// Seconds to wait before the voice starts (for arpeggios / double hits).
    pub delay: f32,
}

impl VoiceSpec {
    pub fn tone(wave: Wave, freq: f32, dur: f32, gain: f32) -> Self {
        VoiceSpec {
            wave,
            freq,
            freq_end: freq,
            attack: 0.002,
            decay: dur * 0.4,
            dur,
            gain,
            noise: 0.0,
            lowpass: 0.0,
            pan: 0.0,
            delay: 0.0,
        }
    }
}

pub struct Voice {
    spec: VoiceSpec,
    phase: f32,
    /// Samples elapsed since the voice was triggered (including delay).
    n: u32,
    delay_n: u32,
    dur_n: u32,
    lp: f32,
    lp_a: f32,
    noise_state: u32,
    sr: f32,
}

impl Voice {
    pub fn new(spec: VoiceSpec, sr: f32, seed: u32) -> Self {
        let lp_a = if spec.lowpass > 0.0 {
            1.0 - (-2.0 * std::f32::consts::PI * spec.lowpass / sr).exp()
        } else {
            1.0
        };
        Voice {
            spec,
            phase: 0.0,
            n: 0,
            delay_n: (spec.delay * sr) as u32,
            dur_n: (spec.dur * sr) as u32,
            lp: 0.0,
            lp_a,
            noise_state: seed | 1,
            sr,
        }
    }

    pub fn finished(&self) -> bool {
        self.n >= self.delay_n + self.dur_n
    }

    /// Add this voice into an interleaved stereo buffer.
    pub fn render_add(&mut self, out: &mut [f32]) {
        let s = self.spec;
        let (gl, gr) = pan_gains(s.pan);
        for frame in out.chunks_exact_mut(2) {
            if self.finished() {
                break;
            }
            let n = self.n;
            self.n += 1;
            if n < self.delay_n {
                continue;
            }
            let t = (n - self.delay_n) as f32 / self.sr;
            let progress = t / s.dur;
            let freq = s.freq + (s.freq_end - s.freq) * progress;
            self.phase = (self.phase + freq / self.sr).fract();
            let p = self.phase;
            let tone = match s.wave {
                // Subtract the pulse's mean (2d - 1) so narrow duties carry no DC
                // thump at the pop rate.
                Wave::Pulse(d) => (if p < d { 1.0 } else { -1.0 }) - (2.0 * d - 1.0),
                Wave::Triangle => 4.0 * (p - 0.5).abs() - 1.0,
                Wave::Sine => (2.0 * std::f32::consts::PI * p).sin(),
            };
            let x = if s.noise > 0.0 {
                // xorshift32 white noise
                let mut v = self.noise_state;
                v ^= v << 13;
                v ^= v >> 17;
                v ^= v << 5;
                self.noise_state = v;
                let nz = (v as f32 / u32::MAX as f32) * 2.0 - 1.0;
                tone * (1.0 - s.noise) + nz * s.noise
            } else {
                tone
            };
            self.lp += self.lp_a * (x - self.lp);
            let env = if t < s.attack {
                t / s.attack
            } else {
                (-(t - s.attack) / s.decay.max(1e-4)).exp()
            };
            // Short fade at the hard stop to avoid clicks.
            let tail = ((s.dur - t) / 0.003).clamp(0.0, 1.0);
            let y = self.lp * env * tail * s.gain;
            frame[0] += y * gl;
            frame[1] += y * gr;
        }
    }
}

fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
    (a.cos(), a.sin())
}

/// Semitones relative to A4 (440 Hz) to Hz.
pub fn note_hz(semitones_from_a4: f32) -> f32 {
    440.0 * 2f32.powf(semitones_from_a4 / 12.0)
}
