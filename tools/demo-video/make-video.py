#!/usr/bin/env python3
"""Mux a CDP screencast recording with tokenpop audio.

usage: make-video.py REC_DIR EVENTS.jsonl OUT.mp4 [--pack blip] [--gain 7] [--tokenpop PATH]

REC_DIR holds frames/ and frames.json written by record.js. Frames and adapter
events share the wall clock, so audio is placed at (first event - first frame).
"""
import argparse, json, os, subprocess, tempfile

ap = argparse.ArgumentParser()
ap.add_argument("rec_dir"); ap.add_argument("events"); ap.add_argument("out")
ap.add_argument("--pack", default="blip"); ap.add_argument("--gain", default="7")
ap.add_argument("--tokenpop", default="tokenpop")
a = ap.parse_args()

meta = json.load(open(os.path.join(a.rec_dir, "frames.json")))
frames = meta["frames"]
first_event = min(json.loads(l)["t"] for l in open(a.events) if l.strip()) / 1000
v0 = frames[0]["ts"]
offset, dur = first_event - v0, frames[-1]["ts"] - v0 + 0.1

with tempfile.TemporaryDirectory() as tmp:
    concat = os.path.join(tmp, "concat.txt")
    with open(concat, "w") as f:
        for x, y in zip(frames, frames[1:]):
            f.write(f"file '{os.path.abspath(os.path.join(a.rec_dir, x['file']))}'\nduration {y['ts'] - x['ts']:.6f}\n")
        last = os.path.abspath(os.path.join(a.rec_dir, frames[-1]["file"]))
        f.write(f"file '{last}'\nduration 0.1\nfile '{last}'\n")
    wav = os.path.join(tmp, "audio.wav")
    subprocess.run([a.tokenpop, "render", "--pack", a.pack, "--gain", a.gain, "-o", wav, f"{a.events}@{offset:.4f}"], check=True)
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0", "-i", concat,
                    "-i", wav, "-map", "0:v", "-map", "1:a",
                    "-vf", "fps=30,scale=1920:1080:flags=lanczos,format=yuv420p",
                    "-c:v", "libx264", "-preset", "slow", "-crf", "18", "-c:a", "aac", "-b:a", "192k",
                    "-t", f"{dur:.3f}", "-movflags", "+faststart", a.out], check=True)
print(f"{a.out}: {dur:.1f}s, audio offset {offset:.3f}s, pack {a.pack}")
