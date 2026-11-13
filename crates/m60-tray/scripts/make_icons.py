"""Regenerates icons/ (needs Pillow): python scripts/make_icons.py, run from crates/m60-tray."""
import os

from PIL import Image, ImageDraw

S = 1024
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
white = (255, 255, 255, 255)
d.rounded_rectangle((40, 40, S - 40, S - 40), radius=230, fill=(24, 119, 242, 255))
d.rectangle((250, 400, 400, 624), fill=white)  # speaker body
d.polygon([(400, 400), (590, 250), (590, 774), (400, 624)], fill=white)  # cone
for r in (170, 290):  # sound waves
    d.arc((630 - r, 512 - r, 630 + r, 512 + r), start=-48, end=48, fill=white, width=46)

os.makedirs("icons", exist_ok=True)
for name, size in (("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256), ("icon.png", 512)):
    img.resize((size, size), Image.LANCZOS).save(f"icons/{name}")
img.resize((256, 256), Image.LANCZOS).save(
    "icons/icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
)
img.save("icons/icon.icns")
