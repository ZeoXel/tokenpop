//! Offline rendering of recorded event streams to WAV, using the exact same
//! engine as the live daemon. Used for demos, pack design, and CI.

use std::io::{self, Write};

use crate::engine::Engine;
use crate::event::{parse_line, Event};
use crate::pack::Pack;

/// One recording placed on the output timeline.
pub struct Track {
    pub events: Vec<Event>,
    /// Start offset on the output timeline, seconds.
    pub offset: f64,
}

/// Load a JSONL recording, re-time it to start at 0 and give its agents a
/// per-track prefix so two recordings never share a voice.
pub fn load_track(text: &str, index: usize, offset: f64, speed: f64) -> Track {
    let mut events: Vec<Event> = text.lines().filter_map(parse_line).collect();
    events.sort_by(|a, b| a.t_ms.total_cmp(&b.t_ms));
    let t0 = events.first().map(|e| e.t_ms).unwrap_or(0.0);
    for e in &mut events {
        e.t_ms = (e.t_ms - t0) / speed;
        e.agent = format!("{index}/{}", e.agent);
    }
    Track { events, offset }
}

/// Render tracks to interleaved stereo f32 frames.
pub fn render(tracks: &[Track], sr: u32, pack: Pack, tail_s: f64) -> Vec<f32> {
    let mut timeline: Vec<(u64, &Event)> = tracks
        .iter()
        .flat_map(|t| {
            t.events
                .iter()
                .map(move |e| (((t.offset + e.t_ms / 1000.0) * sr as f64) as u64, e))
        })
        .collect();
    timeline.sort_by_key(|(f, _)| *f);
    let end = timeline.last().map(|(f, _)| *f).unwrap_or(0) + (tail_s * sr as f64) as u64;

    let mut engine = Engine::new(sr, pack);
    let mut out = vec![0.0f32; end as usize * 2];
    let mut pos = 0usize;
    for (frame, e) in timeline {
        let frame = frame as usize;
        if frame > pos {
            engine.render(&mut out[pos * 2..frame * 2]);
            pos = frame;
        }
        engine.push(e);
    }
    engine.render(&mut out[pos * 2..]);
    out
}

/// Write 16-bit PCM stereo WAV.
pub fn write_wav(w: &mut impl Write, samples: &[f32], sr: u32) -> io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_len).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?; // PCM
    w.write_all(&2u16.to_le_bytes())?; // stereo
    w.write_all(&sr.to_le_bytes())?;
    w.write_all(&(sr * 4).to_le_bytes())?;
    w.write_all(&4u16.to_le_bytes())?;
    w.write_all(&16u16.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data_len.to_le_bytes())?;
    let mut buf = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        buf.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    w.write_all(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_recording_deterministically() {
        let rec = "{\"t\":0,\"kind\":\"prompt\"}\n\
                   {\"t\":100,\"kind\":\"text\",\"chars\":20,\"h\":3}\n\
                   {\"t\":400,\"kind\":\"done\"}\n";
        let a = render(&[load_track(rec, 0, 0.0, 1.0)], 48_000, Pack::Blip, 0.5);
        let b = render(&[load_track(rec, 0, 0.0, 1.0)], 48_000, Pack::Blip, 0.5);
        assert_eq!(a, b);
        assert_eq!(a.len(), (48_000.0 * 0.9) as usize * 2);
        assert!(a.iter().any(|x| x.abs() > 0.01));
        let mut wav = Vec::new();
        write_wav(&mut wav, &a, 48_000).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(wav.len(), 44 + a.len() * 2);
    }
}
