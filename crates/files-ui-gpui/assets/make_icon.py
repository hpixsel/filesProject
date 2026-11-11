"""Draws assets/app.ico (the .exe and window icon) to match icons/app.svg.

Run with Pillow: `py -I assets/make_icon.py` from crates/files-ui-gpui.
"""
from pathlib import Path
from PIL import Image, ImageDraw

S = 1024  # drawn large, then scaled down per icon size
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

# Background: the default accent (hue 217), a soft top-to-bottom gradient.
top, bottom = (96, 165, 250), (37, 99, 235)
grad = Image.new("RGBA", (S, S))
gd = ImageDraw.Draw(grad)
for y in range(S):
    t = y / (S - 1)
    gd.line([(0, y), (S, y)], fill=tuple(round(a + (b - a) * t) for a, b in zip(top, bottom)) + (255,))
mask = Image.new("L", (S, S), 0)
ImageDraw.Draw(mask).rounded_rectangle([40, 40, S - 40, S - 40], radius=220, fill=255)
img.paste(grad, (0, 0), mask)

# White folder: a tab and a body.
white = (255, 255, 255, 255)
d.rounded_rectangle([200, 270, 520, 420], radius=50, fill=white)
d.rounded_rectangle([200, 340, 824, 760], radius=70, fill=white)

# Bolt cut out of the folder in the background blue.
bolt = [(560, 400), (420, 590), (530, 590), (470, 720), (640, 510), (525, 510), (585, 400)]
d.polygon(bolt, fill=(59, 130, 246, 255))

out = Path(__file__).with_name("app.ico")
img.save(out, sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
print(out)
