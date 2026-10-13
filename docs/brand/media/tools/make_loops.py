# Turns each companion's idle video into seamless, transparent animated WebP
# loops at the sizes the app uses for character art (64/128/256) plus 512.
#
#   python make_loops.py            (needs ffmpeg, numpy, scipy, pillow)
#
# The videos have a black background. Only black regions connected to the
# frame's edge are made transparent, so dark details inside a character
# (pupils, nose) stay opaque. The loop plays forward then back, so its last
# frame always meets its first.
import os, subprocess, sys, tempfile
import numpy as np
from PIL import Image
from scipy import ndimage

HERE = os.path.dirname(os.path.abspath(__file__))
MEDIA = os.path.join(HERE, "..")
SOURCES = {"vak": "vakyartha", "mira": "mira", "moss": "moss", "nori": "nori",
           "pip": "pip", "lumi": "lumi", "tavi": "tavi", "beni": "beni"}
SIZES = {512: 12, 256: 12, 128: 8, 64: 8}  # size: frames per second
FPS = 12
BLACK = 28


def cut_background(rgb):
    dark = rgb.max(axis=2) < BLACK
    labels, _ = ndimage.label(dark)
    edge = np.unique(np.concatenate([labels[0], labels[-1], labels[:, 0], labels[:, -1]]))
    bg = np.isin(labels, edge[edge > 0])
    bg = ndimage.binary_dilation(bg, iterations=2)
    alpha = ndimage.gaussian_filter((~bg).astype(np.float32), 0.8)
    return np.dstack([rgb, (alpha * 255).astype(np.uint8)])


def frames_of(video):
    with tempfile.TemporaryDirectory() as tmp:
        subprocess.run(["ffmpeg", "-v", "error", "-i", video, "-vf", f"fps={FPS}",
                        os.path.join(tmp, "f%04d.png")], check=True)
        names = sorted(os.listdir(tmp))
        return [cut_background(np.asarray(Image.open(os.path.join(tmp, n)).convert("RGB")))
                for n in names]


def main():
    out_dir = os.path.join(MEDIA, "loops")
    os.makedirs(out_dir, exist_ok=True)
    only = set(sys.argv[1:])
    for app_id, name in SOURCES.items():
        if only and app_id not in only:
            continue
        video = os.path.join(MEDIA, "video", f"{name}-idle.mp4")
        if not os.path.exists(video):
            print("missing", video)
            continue
        frames = frames_of(video)
        frames = frames + frames[-2:0:-1]
        for size, fps in SIZES.items():
            step = FPS // fps if FPS % fps == 0 else FPS / fps
            picked = [frames[int(i * step)] for i in range(int(len(frames) / step))]
            imgs = [Image.fromarray(f, "RGBA").resize((size, size), Image.LANCZOS) for f in picked]
            path = os.path.join(out_dir, f"{app_id}-idle-{size}.webp")
            imgs[0].save(path, save_all=True, append_images=imgs[1:], duration=round(1000 / fps),
                         loop=0, quality=80 if size >= 256 else 70, method=2, lossless=False)
            print("ok", os.path.relpath(path, MEDIA), os.path.getsize(path))


if __name__ == "__main__":
    main()
