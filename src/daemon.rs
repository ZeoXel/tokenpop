//! Live daemon: accepts JSONL events on a Unix socket and plays them.
//!
//! Lines carrying `"cmd"` are control messages:
//! `{"cmd":"mute"}`, `{"cmd":"unmute"}`, `{"cmd":"pack","name":"mech"}`.

use std::io::{BufRead, BufReader};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::engine::Engine;
use crate::event::{parse_line, Event};
use crate::pack::Pack;

pub enum Msg {
    Event(Event),
    Mute(bool),
    Pack(Pack),
}

/// Default socket path; must match the adapters' logic.
pub fn default_socket() -> PathBuf {
    if let Ok(p) = std::env::var("TOKENPOP_SOCK") {
        return p.into();
    }
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    dir.join(format!("tokenpop-{}.sock", uid()))
}

fn uid() -> String {
    use std::os::unix::fs::MetadataExt;
    std::env::var("HOME")
        .ok()
        .and_then(|h| std::fs::metadata(h).ok())
        .map(|m| m.uid().to_string())
        .unwrap_or_else(|| "user".into())
}

pub fn parse_msg(line: &str) -> Option<Msg> {
    if line.contains("\"cmd\"") {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        return match v.get("cmd")?.as_str()? {
            "mute" => Some(Msg::Mute(true)),
            "unmute" => Some(Msg::Mute(false)),
            "pack" => Pack::from_name(v.get("name")?.as_str()?).map(Msg::Pack),
            _ => None,
        };
    }
    parse_line(line).map(Msg::Event)
}

/// Bind the socket and forward every parsed line to `tx`. Blocks forever.
pub fn serve(path: &Path, tx: Sender<Msg>) -> std::io::Result<()> {
    if path.exists() {
        // A live daemon answers; a stale socket file does not.
        if UnixStream::connect(path).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                format!("tokenpop is already running on {}", path.display()),
            ));
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    eprintln!("tokenpop: listening on {}", path.display());
    for conn in listener.incoming().flatten() {
        let tx = tx.clone();
        thread::spawn(move || {
            for line in BufReader::new(conn).lines().map_while(Result::ok) {
                if let Some(m) = parse_msg(&line) {
                    if tx.send(m).is_err() {
                        return;
                    }
                }
            }
        });
    }
    Ok(())
}

/// Apply pending messages to the engine (called from the audio callback).
pub fn drain(engine: &mut Engine, rx: &Receiver<Msg>) {
    while let Ok(m) = rx.try_recv() {
        match m {
            Msg::Event(e) => engine.push(&e),
            Msg::Mute(b) => engine.muted = b,
            Msg::Pack(p) => engine.set_pack(p),
        }
    }
}

#[cfg(feature = "playback")]
pub fn run(path: &Path, pack: Pack) -> Result<(), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no audio output device found")?;
    let config = device
        .default_output_config()
        .map_err(|e| format!("audio config: {e}"))?;
    if config.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!(
            "unsupported sample format {:?}",
            config.sample_format()
        ));
    }
    let channels = config.channels() as usize;
    let sr = config.sample_rate().0;
    let (tx, rx) = mpsc::channel();
    let mut engine = Engine::new(sr, pack);
    let mut stereo = Vec::new();
    let stream = device
        .build_output_stream(
            &config.into(),
            move |out: &mut [f32], _| {
                drain(&mut engine, &rx);
                let frames = out.len() / channels;
                stereo.resize(frames * 2, 0.0);
                engine.render(&mut stereo);
                for (i, frame) in out.chunks_exact_mut(channels).enumerate() {
                    let (l, r) = (stereo[i * 2], stereo[i * 2 + 1]);
                    match channels {
                        1 => frame[0] = 0.5 * (l + r),
                        _ => {
                            frame[0] = l;
                            frame[1] = r;
                            frame[2..].fill(0.0);
                        }
                    }
                }
            },
            |e| eprintln!("tokenpop: audio error: {e}"),
            None,
        )
        .map_err(|e| format!("audio stream: {e}"))?;
    stream.play().map_err(|e| format!("audio play: {e}"))?;
    eprintln!(
        "tokenpop: playing on {} ({} Hz, pack {})",
        device.name().unwrap_or_default(),
        sr,
        pack.name()
    );
    serve(path, tx).map_err(|e| e.to_string())
}

#[cfg(not(feature = "playback"))]
pub fn run(_path: &Path, _pack: Pack) -> Result<(), String> {
    let _ = (
        mpsc::channel::<Msg>,
        drain as fn(&mut Engine, &Receiver<Msg>),
    );
    Err("this build has no audio playback (built without the `playback` feature)".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn socket_delivers_events_and_commands() {
        let dir = std::env::temp_dir().join(format!("tokenpop-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.sock");
        let (tx, rx) = mpsc::channel();
        let p = path.clone();
        thread::spawn(move || serve(&p, tx));
        let mut conn = (0..100)
            .find_map(|_| {
                thread::sleep(Duration::from_millis(10));
                UnixStream::connect(&path).ok()
            })
            .expect("daemon socket");
        writeln!(conn, r#"{{"kind":"text","chars":4,"agent":"x"}}"#).unwrap();
        writeln!(conn, "garbage").unwrap();
        writeln!(conn, r#"{{"cmd":"pack","name":"mech"}}"#).unwrap();
        writeln!(conn, r#"{{"cmd":"mute"}}"#).unwrap();
        let got: Vec<Msg> = (0..3)
            .map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        assert!(matches!(&got[0], Msg::Event(e) if e.chars == 4 && e.agent == "x"));
        assert!(matches!(got[1], Msg::Pack(Pack::Mech)));
        assert!(matches!(got[2], Msg::Mute(true)));
        // A second daemon on the same live socket refuses to start.
        let (tx2, _rx2) = mpsc::channel();
        assert!(serve(&path, tx2).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
