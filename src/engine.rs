//! The engine: event intake, pop scheduling, mixing.
//!
//! Chunks arrive in bursts (median gap 0 ms, p90 ~18 ms in dsh recordings), so
//! pops are never played 1:1. Each (agent, kind) stream accumulates "credit"
//! from incoming characters and spends it at a capped, jittered rate:
//!
//! * fast output saturates the rate cap -> dense, steady pops;
//! * slow output pops as it arrives;
//! * credit is capped and expires quickly, so sound never lags behind text.
//!
//! The same code drives the live daemon and offline WAV rendering.

use std::collections::HashMap;

use crate::event::{Event, Kind};
use crate::pack::{self, AgentVoice, Pack};
use crate::synth::Voice;

/// Scheduler resolution in frames (~1.3 ms at 48 kHz).
const TICK: usize = 64;
/// Max simultaneous voices; the oldest is dropped beyond this.
const MAX_VOICES: usize = 32;
/// Credit expires after this much silence from a stream, seconds.
const IDLE_RESET: f32 = 0.25;
/// Never queue more than this many seconds of pops.
const MAX_BACKLOG: f32 = 0.3;

struct Stream {
    credit: f32,
    last_event: u64,
    next_pop: u64,
    count: u32,
    last_h: u8,
    rng: u32,
}

fn xorshift(state: &mut u32) -> f32 {
    let mut v = *state;
    v ^= v << 13;
    v ^= v >> 17;
    v ^= v << 5;
    *state = v;
    (v >> 8) as f32 / (1u32 << 24) as f32
}

struct AgentState {
    voice: AgentVoice,
    streams: HashMap<Kind, Stream>,
}

pub struct Engine {
    sr: f32,
    pack: Pack,
    agents: HashMap<String, AgentState>,
    voices: Vec<Voice>,
    /// Current time in frames.
    now: u64,
    rng: u32,
    pub master: f32,
    pub muted: bool,
    /// Total stream pops emitted (diagnostics / tests).
    pub pops: u64,
    /// DC blocker state per channel: (previous input, previous output).
    dc: [(f32, f32); 2],
}

impl Engine {
    pub fn new(sr: u32, pack: Pack) -> Self {
        Engine {
            sr: sr as f32,
            pack,
            agents: HashMap::new(),
            voices: Vec::new(),
            now: 0,
            rng: 0x9e37_79b9,
            master: 0.8,
            muted: false,
            pops: 0,
            dc: [(0.0, 0.0); 2],
        }
    }

    pub fn set_pack(&mut self, pack: Pack) {
        self.pack = pack;
    }

    pub fn now_frames(&self) -> u64 {
        self.now
    }

    fn rand(&mut self) -> f32 {
        xorshift(&mut self.rng)
    }

    fn agent(&mut self, name: &str) -> &mut AgentState {
        let n = self.agents.len();
        self.agents
            .entry(name.to_string())
            .or_insert_with(|| AgentState {
                voice: pack::agent_voice(n),
                streams: HashMap::new(),
            })
    }

    /// Feed one event at the current time.
    pub fn push(&mut self, e: &Event) {
        if self.muted {
            return;
        }
        let now = self.now;
        let (sr, pk) = (self.sr, self.pack);
        if e.kind.is_stream() {
            let p = pack::pacing(pk, e.kind);
            let seed = 0x1234_5678
                ^ (e.kind as u32).wrapping_mul(0x85eb_ca6b)
                ^ (self.agents.len() as u32);
            let st = self
                .agent(&e.agent)
                .streams
                .entry(e.kind)
                .or_insert(Stream {
                    credit: 0.0,
                    last_event: 0,
                    next_pop: 0,
                    count: 0,
                    last_h: 0,
                    rng: seed | 1,
                });
            let idle = now.saturating_sub(st.last_event) as f32 / sr > IDLE_RESET;
            if idle || st.count == 0 {
                // A stream (re)starting answers immediately with one pop.
                st.credit = st.credit.max(1.0);
            }
            st.credit += e.chars as f32 / p.chars_per_pop;
            st.credit = st.credit.min(1.0 + p.max_hz * MAX_BACKLOG);
            st.last_event = now;
            st.last_h = e.h;
        } else {
            let v = self.agent(&e.agent).voice;
            for spec in pack::cue(pk, e.kind, v) {
                self.spawn(spec);
            }
        }
    }

    fn spawn(&mut self, spec: crate::synth::VoiceSpec) {
        if self.voices.len() >= MAX_VOICES {
            self.voices.remove(0);
        }
        let seed = (self.rand() * u32::MAX as f32) as u32;
        self.voices.push(Voice::new(spec, self.sr, seed));
    }

    /// Run the pop scheduler for the current tick.
    fn schedule(&mut self) {
        let now = self.now;
        let (sr, pk) = (self.sr, self.pack);
        let mut specs = Vec::new();
        for st in self.agents.values_mut() {
            let voice = st.voice;
            for (&kind, s) in st.streams.iter_mut() {
                if now.saturating_sub(s.last_event) as f32 / sr > IDLE_RESET {
                    s.credit = 0.0;
                    continue;
                }
                if s.credit < 1.0 || now < s.next_pop {
                    continue;
                }
                s.credit -= 1.0;
                s.count += 1;
                let p = pack::pacing(pk, kind);
                let j = xorshift(&mut s.rng);
                let interval = sr / p.max_hz * (1.0 + p.jitter * (2.0 * j - 1.0));
                s.next_pop = now + interval as u64;
                let sel = (s.last_h as u32) ^ (s.count.wrapping_mul(2_654_435_761) >> 7);
                specs.push(pack::stream_pop(pk, kind, voice, sel, xorshift(&mut s.rng)));
            }
        }
        self.pops += specs.len() as u64;
        for spec in specs {
            self.spawn(spec);
        }
    }

    /// Render interleaved stereo frames, advancing time.
    pub fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        if self.muted {
            // Mute is instant: drop ringing voices and pending credit.
            self.voices.clear();
            self.agents
                .values_mut()
                .flat_map(|a| a.streams.values_mut())
                .for_each(|s| s.credit = 0.0);
            self.now += (out.len() / 2) as u64;
            return;
        }
        for block in out.chunks_mut(TICK * 2) {
            self.schedule();
            for v in self.voices.iter_mut() {
                v.render_add(block);
            }
            self.voices.retain(|v| !v.finished());
            for frame in block.chunks_exact_mut(2) {
                for (x, dc) in frame.iter_mut().zip(self.dc.iter_mut()) {
                    // One-pole DC blocker (~15 Hz), then soft clip.
                    let y = *x - dc.0 + 0.998 * dc.1;
                    *dc = (*x, y);
                    *x = (y * self.master).tanh();
                }
            }
            self.now += (block.len() / 2) as u64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: Kind, chars: u32) -> Event {
        Event {
            t_ms: 0.0,
            agent: "a".into(),
            kind,
            chars,
            h: 7,
        }
    }

    /// Feed `chunks_per_s` chunks of `chars` for `secs`, return pops/s.
    fn pop_rate(chunks_per_s: f32, chars: u32, secs: f32) -> f32 {
        let sr = 48_000;
        let mut e = Engine::new(sr, Pack::Blip);
        let step = (sr as f32 / chunks_per_s) as usize;
        let total = (secs * sr as f32) as usize;
        let mut buf = vec![0.0; step * 2];
        let mut t = 0;
        while t < total {
            e.push(&ev(Kind::Text, chars));
            e.render(&mut buf);
            t += step;
        }
        e.pops as f32 / secs
    }

    #[test]
    fn fast_streams_hit_the_rate_cap() {
        // 400 chars/s would be 80 pops/s at 5 chars/pop; the cap is 20 Hz.
        let r = pop_rate(100.0, 4, 3.0);
        assert!((16.0..=24.0).contains(&r), "rate {r}");
    }

    #[test]
    fn slow_streams_follow_the_text() {
        // 25 chars/s -> about 5 pops/s.
        let r = pop_rate(5.0, 5, 4.0);
        assert!((4.0..=7.0).contains(&r), "rate {r}");
    }

    #[test]
    fn sound_stops_soon_after_text_stops() {
        let sr = 48_000;
        let mut e = Engine::new(sr, Pack::Blip);
        // A huge burst: far more credit than the backlog cap allows.
        e.push(&ev(Kind::Text, 10_000));
        let mut buf = vec![0.0; sr as usize * 2 * 2];
        e.render(&mut buf);
        // At most ~0.3 s of backlog at 20 Hz (+ the immediate first pop).
        assert!(e.pops <= 9, "pops {}", e.pops);
        // The last second is silent.
        assert!(buf[sr as usize * 2..].iter().all(|x| x.abs() < 1e-6));
    }

    #[test]
    fn output_has_no_dc_offset() {
        let sr = 48_000;
        let mut e = Engine::new(sr, Pack::Blip);
        let mut buf = vec![0.0; 960 * 2];
        let mut sum = 0.0f64;
        let mut n = 0;
        for _ in 0..100 {
            e.push(&ev(Kind::Text, 8));
            e.render(&mut buf);
            sum += buf.iter().map(|&x| x as f64).sum::<f64>();
            n += buf.len();
        }
        let mean = sum / n as f64;
        assert!(mean.abs() < 1e-3, "mean {mean}");
    }

    #[test]
    fn cues_make_sound_and_mute_silences() {
        let mut e = Engine::new(48_000, Pack::Blip);
        e.push(&ev(Kind::Done, 0));
        let mut buf = vec![0.0; 48_000];
        e.render(&mut buf);
        assert!(buf.iter().any(|x| x.abs() > 0.01));
        e.muted = true;
        e.push(&ev(Kind::Done, 0));
        e.render(&mut buf);
        assert!(buf.iter().all(|x| x.abs() < 1e-6));
    }
}
