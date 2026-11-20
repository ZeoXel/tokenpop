//! Built-in sound packs. A pack decides (a) how fast each stream kind may pop
//! and (b) what each pop or cue sounds like. External `pack.toml` packs come later.

use crate::event::Kind;
use crate::synth::{note_hz, VoiceSpec, Wave};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pack {
    /// Cute dialogue blips (Undertale / Animal Crossing style). Default.
    Blip,
    /// Soft mechanical keyboard.
    Mech,
    /// 8-bit chiptune.
    Chip,
}

impl Pack {
    pub const ALL: [Pack; 3] = [Pack::Blip, Pack::Mech, Pack::Chip];

    pub fn name(self) -> &'static str {
        match self {
            Pack::Blip => "blip",
            Pack::Mech => "mech",
            Pack::Chip => "chip",
        }
    }

    pub fn from_name(s: &str) -> Option<Pack> {
        Pack::ALL.into_iter().find(|p| p.name() == s)
    }
}

/// Pacing for one stream kind.
#[derive(Debug, Clone, Copy)]
pub struct Pacing {
    /// Characters of streamed text per pop.
    pub chars_per_pop: f32,
    /// Hard ceiling on pops per second.
    pub max_hz: f32,
    /// Random +/- fraction applied to each interval so it never sounds mechanical.
    pub jitter: f32,
}

pub fn pacing(pack: Pack, kind: Kind) -> Pacing {
    let (cpp, hz) = match (pack, kind) {
        (Pack::Mech, Kind::Text) => (4.0, 24.0),
        (_, Kind::Text) => (5.0, 20.0),
        (_, Kind::Reasoning) => (12.0, 7.0),
        (_, Kind::ToolArgs) => (6.0, 16.0),
        _ => (1.0, 1.0),
    };
    Pacing {
        chars_per_pop: cpp,
        max_hz: hz,
        jitter: 0.25,
    }
}

/// Two octaves of a major pentatonic scale: any sequence sounds pleasant.
const PENTA: [f32; 8] = [0.0, 2.0, 4.0, 7.0, 9.0, 12.0, 14.0, 16.0];

/// Per-agent voice: root note (semitones from A4) and stereo position.
#[derive(Debug, Clone, Copy)]
pub struct AgentVoice {
    pub root: f32,
    pub pan: f32,
}

pub fn agent_voice(index: usize) -> AgentVoice {
    const ROOTS: [f32; 5] = [3.0, -2.0, 5.0, 0.0, -4.0]; // C5, G4, D5, A4, F4
    const PANS: [f32; 5] = [0.0, -0.55, 0.55, -0.3, 0.3];
    AgentVoice {
        root: ROOTS[index % 5],
        pan: PANS[index % 5],
    }
}

/// One pop for a streaming kind. `sel` varies pitch (from the content hash and
/// a counter); `r` is a uniform random number in [0, 1).
pub fn stream_pop(pack: Pack, kind: Kind, v: AgentVoice, sel: u32, r: f32) -> VoiceSpec {
    let step = PENTA[(sel % PENTA.len() as u32) as usize];
    match (pack, kind) {
        (Pack::Blip, Kind::Text) => {
            let f = note_hz(v.root + step);
            VoiceSpec {
                freq_end: f * 0.97,
                decay: 0.018,
                lowpass: 3800.0,
                pan: v.pan,
                ..VoiceSpec::tone(Wave::Pulse(0.25), f, 0.045, 0.20)
            }
        }
        (Pack::Chip, Kind::Text) => VoiceSpec {
            decay: 0.03,
            pan: v.pan,
            ..VoiceSpec::tone(Wave::Pulse(0.5), note_hz(v.root + step), 0.035, 0.12)
        },
        (Pack::Mech, Kind::Text | Kind::ToolArgs) => {
            // Key click: bright noise burst with a little random brightness/level.
            let heavy = kind == Kind::ToolArgs;
            VoiceSpec {
                noise: 0.9,
                attack: 0.0005,
                decay: if heavy { 0.012 } else { 0.008 },
                lowpass: 5000.0 + 3000.0 * r,
                pan: v.pan + (r - 0.5) * 0.2,
                ..VoiceSpec::tone(
                    Wave::Triangle,
                    if heavy { 180.0 } else { 260.0 } * (0.9 + 0.2 * r),
                    0.035,
                    if heavy { 0.30 } else { 0.22 } * (0.8 + 0.4 * r),
                )
            }
        }
        (_, Kind::Reasoning) => {
            // Thinking: soft, low, sparse. An octave below the agent's root.
            let f = note_hz(v.root - 12.0 + step);
            VoiceSpec {
                attack: 0.006,
                decay: 0.05,
                pan: v.pan,
                ..VoiceSpec::tone(Wave::Sine, f, 0.11, 0.16)
            }
        }
        (_, _) => {
            // Tool arguments in blip/chip: a crisp, higher "typing" tick.
            let f = note_hz(v.root + 12.0 + step);
            VoiceSpec {
                noise: 0.35,
                decay: 0.01,
                lowpass: 6000.0,
                pan: v.pan,
                ..VoiceSpec::tone(Wave::Triangle, f, 0.03, 0.12)
            }
        }
    }
}

/// One-shot cue for a non-streaming event.
pub fn cue(pack: Pack, kind: Kind, v: AgentVoice) -> Vec<VoiceSpec> {
    let chip = pack == Pack::Chip;
    let tone = |semi: f32, dur: f32, gain: f32, delay: f32| VoiceSpec {
        delay,
        pan: v.pan,
        decay: dur * 0.45,
        ..VoiceSpec::tone(
            if chip {
                Wave::Pulse(0.5)
            } else {
                Wave::Triangle
            },
            note_hz(v.root + semi),
            dur,
            if chip { gain * 0.6 } else { gain },
        )
    };
    match kind {
        Kind::Prompt => vec![VoiceSpec {
            freq_end: note_hz(v.root + 7.0),
            pan: v.pan,
            attack: 0.01,
            decay: 0.05,
            ..VoiceSpec::tone(Wave::Sine, note_hz(v.root - 5.0), 0.09, 0.25)
        }],
        Kind::ToolStart => vec![
            // Low "thunk" plus a click: the agent picks up a tool.
            VoiceSpec {
                freq_end: 70.0,
                decay: 0.04,
                pan: v.pan,
                ..VoiceSpec::tone(Wave::Sine, 160.0, 0.09, 0.45)
            },
            VoiceSpec {
                noise: 1.0,
                decay: 0.006,
                lowpass: 4000.0,
                pan: v.pan,
                ..VoiceSpec::tone(Wave::Sine, 1.0, 0.02, 0.25)
            },
        ],
        Kind::ToolEnd => vec![tone(7.0, 0.06, 0.22, 0.0), tone(12.0, 0.09, 0.22, 0.055)],
        Kind::ToolError | Kind::Error => vec![
            // Detuned tritone buzz.
            VoiceSpec {
                lowpass: 2200.0,
                decay: 0.12,
                pan: v.pan,
                ..VoiceSpec::tone(Wave::Pulse(0.5), note_hz(v.root - 12.0), 0.26, 0.16)
            },
            VoiceSpec {
                lowpass: 2200.0,
                decay: 0.12,
                pan: v.pan,
                ..VoiceSpec::tone(Wave::Pulse(0.5), note_hz(v.root - 6.0) * 1.01, 0.26, 0.14)
            },
        ],
        Kind::Approval => {
            // Two bright bell hits: loud enough to pull you back to the window.
            let bell = |delay: f32| {
                [
                    VoiceSpec {
                        delay,
                        decay: 0.25,
                        ..VoiceSpec::tone(Wave::Sine, note_hz(v.root + 24.0), 0.6, 0.30)
                    },
                    VoiceSpec {
                        delay,
                        decay: 0.12,
                        ..VoiceSpec::tone(Wave::Sine, note_hz(v.root + 24.0) * 2.76, 0.4, 0.10)
                    },
                ]
            };
            bell(0.0).into_iter().chain(bell(0.2)).collect()
        }
        Kind::Done => [0.0, 4.0, 7.0, 12.0]
            .iter()
            .enumerate()
            .map(|(i, s)| tone(*s, if i == 3 { 0.35 } else { 0.12 }, 0.2, i as f32 * 0.07))
            .collect(),
        Kind::Text | Kind::Reasoning | Kind::ToolArgs => vec![],
    }
}
