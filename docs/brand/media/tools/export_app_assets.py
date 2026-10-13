# Writes the app's copies of the brand media into crates/vak-client-ui/public:
#   assets/brand/dimensional/loops/<id>-<tier>.webp  (64/128 at 8 fps)
#   assets/sounds/<family>/<moment>.mp3              (trimmed, levelled)
#   assets/sounds/companions/<id>.mp3                (tuned by tune_companions.py)
#
#   python export_app_assets.py   (needs ffmpeg, numpy, scipy, pillow)
#
# The 512 loops and the raw takes stay in docs/brand/media; the app ships
# only what it plays. Sounds lose leading silence, stop by 1.2 s with a
# short fade, and are levelled to about -22 LUFS. A take that is silent
# (the model sometimes returns one) is never exported: the moment plays its
# declared stand-in from the same family, or the export stops.
import os, subprocess
from PIL import Image
from make_loops import SOURCES, frames_of, MEDIA

PUBLIC = os.path.join(MEDIA, "..", "..", "..", "crates", "vak-client-ui", "public")
LOOPS = os.path.join(PUBLIC, "assets", "brand", "dimensional", "loops")
SOUNDS = os.path.join(PUBLIC, "assets", "sounds")
TIERS = {64: 70, 128: 70}  # tier: webp quality; active marks are small, so no 256
FPS = 8
FAMILIES = ("glass", "santoor", "brass", "wood", "pure")
MOMENTS = ("listening", "turn-finished", "approval-needed", "error", "notification")
SILENT_BELOW_DB = -45.0
# wood/listening came back silent (peak -54.7 dB); until a new take exists it
# plays the family's notification tap.
STAND_INS = {("wood", "listening"): "notification"}


def peak_db(path):
    out = subprocess.run(["ffmpeg", "-hide_banner", "-i", path, "-af", "volumedetect", "-f", "null", "-"],
                         capture_output=True, text=True, check=True).stderr
    return float(out.split("max_volume:")[1].split("dB")[0])


def loops():
    os.makedirs(LOOPS, exist_ok=True)
    for app_id, name in SOURCES.items():
        frames = frames_of(os.path.join(MEDIA, "video", f"{name}-idle.mp4"))[:: 12 // FPS]
        frames = frames + frames[-2:0:-1]
        for tier, quality in TIERS.items():
            imgs = [Image.fromarray(f, "RGBA").resize((tier, tier), Image.LANCZOS) for f in frames]
            path = os.path.join(LOOPS, f"{app_id}-{tier}.webp")
            imgs[0].save(path, save_all=True, append_images=imgs[1:], duration=1000 // FPS,
                         loop=0, quality=quality, method=2)
            print("loop", os.path.relpath(path, PUBLIC), os.path.getsize(path) // 1024, "KB")


def sounds():
    chain = ("silenceremove=start_periods=1:start_threshold=-45dB,atrim=0:1.2,"
             "afade=t=out:st=1.0:d=0.2,loudnorm=I=-22:TP=-3:LRA=7")
    for family in FAMILIES:
        os.makedirs(os.path.join(SOUNDS, family), exist_ok=True)
        for moment in MOMENTS:
            src = os.path.join(MEDIA, "sounds", "system", family, f"{moment}.mp3")
            if peak_db(src) < SILENT_BELOW_DB:
                stand_in = STAND_INS.get((family, moment))
                if not stand_in:
                    raise SystemExit(f"{family}/{moment}.mp3 is silent and has no stand-in")
                src = os.path.join(MEDIA, "sounds", "system", family, f"{stand_in}.mp3")
                print("silent take:", family, moment, "plays", stand_in)
            out = os.path.join(SOUNDS, family, f"{moment}.mp3")
            subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", src, "-af", chain,
                            "-ac", "1", "-ar", "44100", "-b:a", "96k", out], check=True)
            if peak_db(out) < SILENT_BELOW_DB:
                raise SystemExit(f"{os.path.relpath(out, PUBLIC)} was written silent")
            print("sound", os.path.relpath(out, PUBLIC), os.path.getsize(out) // 1024, "KB")


def companions():
    import shutil
    out = os.path.join(SOUNDS, "companions")
    os.makedirs(out, exist_ok=True)
    for app_id in SOURCES:
        shutil.copyfile(os.path.join(MEDIA, "sounds", "companions", f"{app_id}.mp3"), os.path.join(out, f"{app_id}.mp3"))
        print("companion", app_id)


if __name__ == "__main__":
    import sys
    steps = {"loops": loops, "sounds": sounds, "companions": companions}
    for name in sys.argv[1:] or steps:
        steps[name]()
