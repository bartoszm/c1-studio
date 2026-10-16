"""Encodes recorded frames (frame-NNNN.jpg + times.json) as an H.264 MP4, each frame held
for as long as it was on screen. Needs ffmpeg on PATH (winget install Gyan.FFmpeg).

usage: python to_mp4.py <frames dir> <out.mp4> [--fps 30] [--crf 20]
"""
import argparse
import json
import shutil
import subprocess
from pathlib import Path

ap = argparse.ArgumentParser()
ap.add_argument("frames", type=Path)
ap.add_argument("out", type=Path)
ap.add_argument("--fps", type=int, default=30)
ap.add_argument("--crf", type=int, default=20)
a = ap.parse_args()

ffmpeg = shutil.which("ffmpeg")
if not ffmpeg:
    raise SystemExit("ffmpeg is not on PATH: winget install Gyan.FFmpeg")

times = json.loads((a.frames / "times.json").read_text())
files = sorted(a.frames.glob("frame-*.jpg"))
assert files and len(files) == len(times), (len(files), len(times))

# The concat demuxer holds each image for its `duration`; the last one is listed twice,
# as the demuxer ignores the final duration.
lines = ["ffconcat version 1.0"]
for k, f in enumerate(files):
    hold = (times[k + 1] - times[k]) if k + 1 < len(times) else 1.7
    lines += [f"file '{f.resolve().as_posix()}'", f"duration {hold:.4f}"]
lines.append(f"file '{files[-1].resolve().as_posix()}'")
listing = a.frames / "frames.ffconcat"
listing.write_text("\n".join(lines) + "\n")

a.out.parent.mkdir(parents=True, exist_ok=True)
subprocess.run(
    [ffmpeg, "-y", "-loglevel", "error", "-safe", "0", "-f", "concat", "-i", str(listing),
     "-vf", f"fps={a.fps},scale=trunc(iw/2)*2:trunc(ih/2)*2", "-c:v", "libx264", "-preset", "slow",
     "-crf", str(a.crf), "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(a.out)],
    check=True,
)
print(f"{a.out}: {a.out.stat().st_size / 1e6:.1f} MB")
