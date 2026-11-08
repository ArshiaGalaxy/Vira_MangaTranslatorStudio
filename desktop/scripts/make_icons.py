# Generates the base app icon (src-tauri/icons/base-1024.png) with Pillow.
# The Tauri CLI (`tauri icon`) then derives all platform-specific sizes.
from PIL import Image, ImageDraw
import math, os

SIZE = 1024
img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))

# Rounded-square background with a diagonal purple gradient.
c1, c2 = (106, 74, 255), (146, 110, 255)
for y in range(SIZE):
    t = y / SIZE
    d.line([(0, y), (SIZE, y)], fill=lerp(c1, c2, t) + (255,))
mask = Image.new("L", (SIZE, SIZE), 0)
md = ImageDraw.Draw(mask)
R = 190
md.rounded_rectangle([16, 16, SIZE - 16, SIZE - 16], radius=R, fill=255)
img.putalpha(mask)
d = ImageDraw.Draw(img)

# Speech-bubble/tail accents: a warm dot (sun) top-right — manga flavour.
d.ellipse([SIZE - 250, 160, SIZE - 130, 280], fill=(255, 209, 102, 255))

# Stylized white "V" with a pen-nib serif feel. Drawn as two tapered strokes.
v_top_y = 330
v_bottom_y = 680
mid_x = SIZE / 2 - 40
stroke_w_left_top, stroke_w_right_top = 150, 96

# Left arm: thick, from top-left to bottom-mid.
d.polygon([(300, v_top_y), (300 + stroke_w_left_top, v_top_y),
           (mid_x + 60, v_bottom_y), (mid_x - 60, v_bottom_y)], fill=(255, 255, 255, 255))
# Right arm: thinner, from top-right to bottom-mid (taper).
d.polygon([(760, v_top_y), (760 - stroke_w_right_top, v_top_y),
           (mid_x + 20, v_bottom_y - 4), (mid_x - 20, v_bottom_y - 4)],
          fill=(255, 255, 255, 235))

icons_dir = os.path.join(os.path.dirname(__file__), "..", "src-tauri", "icons")
os.makedirs(icons_dir, exist_ok=True)
out = os.path.join(icons_dir, "base-1024.png")
img.save(out)

# Also save the small squares Tauri's config references directly.
img.resize((128, 128), Image.LANCZOS).save(os.path.join(icons_dir, "128x128.png"))
img.resize((32, 32), Image.LANCZOS).save(os.path.join(icons_dir, "32x32.png"))
print("base icon written:", out)
