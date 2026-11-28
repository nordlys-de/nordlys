"""scripts/make-wallpaper.py (needs Pillow)."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from support import REPO

try:
    from PIL import Image
except ImportError:  # pragma: no cover
    Image = None


@unittest.skipIf(Image is None, "Pillow isn't installed")
class Wallpaper(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory(prefix="nlw-")
        self.addCleanup(tmp.cleanup)
        self.tmp = Path(tmp.name)

    def render(self, *args):
        subprocess.run(
            [sys.executable, "-B", str(REPO / "scripts/make-wallpaper.py"), *map(str, args)],
            cwd=self.tmp, check=True, timeout=120,
        )

    def near(self, pixel, wanted, slack):
        self.assertTrue(
            all(abs(a - b) <= slack for a, b in zip(pixel, wanted)),
            f"{pixel} is not within {slack} of {wanted}",
        )

    def test_it_draws_the_orbs_on_near_black_at_any_size(self):
        self.render("out.png", 288, 162)
        with Image.open(self.tmp / "out.png") as image:
            self.assertEqual((image.size, image.mode), ((288, 162), "RGB"))
            pixel = lambda x, y: image.getpixel((x, y))
            # The design is 1440 wide; this is a fifth of it.
            empty = pixel(270, 8)
            blue = pixel(40, 22)
            violet = pixel(236, 148)
        self.near(empty, (10, 13, 19), 14)
        self.assertGreater(blue[2], empty[2] + 30, "the blue orb, top left")
        self.assertGreater(violet[2], empty[2] + 30, "the violet orb, bottom right")
        self.assertGreater(violet[0], blue[0], "violet is redder than blue")

    def test_the_default_name_is_a_4k_png(self):
        self.render()
        with Image.open(self.tmp / "wallpaper.png") as image:
            self.assertEqual((image.size, image.format), ((3840, 2160), "PNG"))

    def test_a_width_alone_is_ignored(self):
        self.render("w.jpg", 100)
        with Image.open(self.tmp / "w.jpg") as image:
            self.assertEqual((image.size, image.format), ((3840, 2160), "JPEG"))

    def test_the_shipped_wallpaper_is_a_large_16_by_9_jpeg(self):
        with Image.open(REPO / "session/wallpaper.jpg") as image:
            width, height = image.size
            self.assertEqual(image.format, "JPEG")
        self.assertGreaterEqual(width, 2560)
        self.assertEqual(width * 9, height * 16)


if __name__ == "__main__":
    unittest.main()
