"""Turns recorded frames (frame-NNNN.jpg + times.json) into an animated GIF at a steady rate.

usage: python assemble.py <frames dir> <out.gif> [--fps 10] [--width 960] [--colors 255]
"""
import argparse
import json
from pathlib import Path

from PIL import Image

ap = argparse.ArgumentParser()
ap.add_argument("frames", type=Path)
ap.add_argument("out", type=Path)
ap.add_argument("--fps", type=float, default=10)
ap.add_argument("--width", type=int, default=960)
ap.add_argument("--colors", type=int, default=255)
a = ap.parse_args()

times = json.loads((a.frames / "times.json").read_text())
files = sorted(a.frames.glob("frame-*.jpg"))
assert files and len(files) == len(times), (len(files), len(times))

# The screencast sends a frame only when the page changes: hold each until the next.
step = 1 / a.fps
start, end = times[0], times[-1] + 0.5
picks, i, t = [], 0, start
while t <= end:
    while i + 1 < len(times) and times[i + 1] <= t:
        i += 1
    picks.append(i)
    t += step

images, durations = [], []
for idx in picks:
    if images and idx == images[-1][0]:
        durations[-1] += round(1000 * step)
        continue
    img = Image.open(files[idx]).convert("RGB")
    h = round(img.height * a.width / img.width)
    images.append((idx, img.resize((a.width, h), Image.LANCZOS)))
    durations.append(round(1000 * step))
# Hold the last frame a moment before the loop starts again.
durations[-1] += 1200

# One palette for the whole clip keeps colours from flickering between frames.
n = min(8, len(images))
sample = images[:: max(1, len(images) // n)][:n]
w, h = sample[0][1].size
sheet = Image.new("RGB", (w, h * len(sample)))
for k, (_, img) in enumerate(sample):
    sheet.paste(img, (0, k * h))
palette = sheet.quantize(colors=a.colors, method=Image.Quantize.MEDIANCUT)
frames = [img.quantize(palette=palette, dither=Image.Dither.FLOYDSTEINBERG) for _, img in images]

a.out.parent.mkdir(parents=True, exist_ok=True)
frames[0].save(a.out, save_all=True, append_images=frames[1:], duration=durations, loop=0, optimize=True, disposal=1)
print(f"{a.out}: {len(frames)} frames, {sum(durations) / 1000:.1f} s, {a.out.stat().st_size / 1e6:.1f} MB")
