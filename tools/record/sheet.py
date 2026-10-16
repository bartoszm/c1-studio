"""A contact sheet of a GIF's frames, for checking a clip. usage: python sheet.py in.gif out.png"""
import sys

from PIL import Image, ImageSequence

gif = Image.open(sys.argv[1])
frames = [f.convert("RGB") for f in ImageSequence.Iterator(gif)]
pick = frames[:: max(1, len(frames) // 16)][:16]
w, h = 360, round(frames[0].height * 360 / frames[0].width)
sheet = Image.new("RGB", (w * 4, h * ((len(pick) + 3) // 4)))
for n, f in enumerate(pick):
    sheet.paste(f.resize((w, h)), ((n % 4) * w, (n // 4) * h))
sheet.save(sys.argv[2])
print(len(frames), "frames,", len(pick), "shown")
