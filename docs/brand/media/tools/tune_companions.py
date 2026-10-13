# Makes each companion's signature sound play exactly its two cue notes
# (`cue` in crates/vak-client-ui/src/agentGlyph.ts), keeping the character's
# own instrument from its generated take.
#
#   python tune_companions.py      (needs ffmpeg, numpy, scipy)
#
# The sound-effects model does not follow requested pitches, so the takes in
# sounds/companions/<character>-<take>.mp3 have the right instrument and the
# wrong notes. A clean single note of each character's instrument was
# generated for this (sounds/companions/source/). This cuts the clearest note, measures its
# fundamental, and repitches it to each cue note by resampling, as a sampler
# does. The two notes are joined into one short sound, and the result is
# measured again: a note more than 20 cents off fails the run.
import glob, math, os, subprocess, sys
import numpy as np
from scipy import signal

HERE = os.path.dirname(os.path.abspath(__file__))
MEDIA = os.path.join(HERE, "..")
SR = 44100
CUES = {"vak": (392, 587), "mira": (392, 523), "moss": (330, 392), "nori": (440, 587), "pip": (523, 659),
        "lumi": (494, 740), "tavi": (349, 466), "beni": (294, 392)}
GAP = 0.16          # seconds between the two notes
NOTE = 0.5          # longest a note rings before it is faded out
TOLERANCE = 20      # cents


def decode(path):
    raw = subprocess.run(["ffmpeg", "-v", "error", "-i", path, "-ac", "1", "-ar", str(SR),
                          "-f", "f32le", "-"], capture_output=True, check=True).stdout
    return np.frombuffer(raw, np.float32).copy()


def onsets(x):
    env = np.convolve(np.abs(x), np.ones(256) / 256, "same")
    rise = np.diff(env, prepend=0)
    peaks, _ = signal.find_peaks(rise, height=rise.max() * 0.3, distance=int(0.08 * SR))
    return peaks, env


def yin(seg, low=150, high=4000, threshold=0.15):
    """YIN pitch (de Cheveigné and Kawahara, 2002) of one steady segment."""
    seg = seg.astype(np.float64)
    lo, hi = int(SR / high), int(SR / low)
    if len(seg) < 2 * hi:
        return None
    w = len(seg) - hi
    energy = np.cumsum(np.concatenate([[0], seg ** 2]))
    corr = signal.fftconvolve(seg, seg[::-1], "full")[len(seg) - 1:]
    d = np.array([energy[w] - energy[0] + energy[tau + w] - energy[tau] - 2 * corr_window(seg, tau, w)
                  for tau in range(hi + 1)])
    cmnd = np.ones_like(d)
    cmnd[1:] = d[1:] * np.arange(1, hi + 1) / np.maximum(np.cumsum(d[1:]), 1e-12)
    below = np.where(cmnd[lo:hi] < threshold)[0]
    tau = lo + (below[0] if len(below) else int(np.argmin(cmnd[lo:hi])))
    while tau + 1 < hi and cmnd[tau + 1] < cmnd[tau]:
        tau += 1
    if 0 < tau < hi:
        a, b, c = cmnd[tau - 1], cmnd[tau], cmnd[tau + 1]
        denom = a - 2 * b + c
        tau = tau + (0.5 * (a - c) / denom if denom else 0)
    return SR / tau


def corr_window(seg, tau, w):
    return float(np.dot(seg[:w], seg[tau:tau + w]))


WINDOW = 2048


def fundamental(note, at=0.015):
    start = int(at * SR)
    return yin(note[start:start + WINDOW])


def peak_at(seg, target):
    """True when the spectrum has a strong partial within 30 cents of target."""
    n = 1 << 16
    spec = np.abs(np.fft.rfft(seg * np.hanning(len(seg)), n=n))
    freqs = np.fft.rfftfreq(n, 1 / SR)
    band = (freqs > 60) & (freqs < 4000)
    for k in (1,):
        near = (freqs > k * target * 2 ** (-30 / 1200)) & (freqs < k * target * 2 ** (30 / 1200))
        if spec[near].max() > 0.25 * spec[band].max():
            return True
    return False


def cents(f, target):
    return 1200 * math.log2(f / target)


def candidate_notes(character):
    """Every steady struck note across the character's sources, cleanest first."""
    found = []
    # Vakyartha's instrument is the brand's glass marimba: its one clean note
    # is the Glass family's notification.
    sources = ([os.path.join(MEDIA, "sounds", "system", "glass", "notification.mp3")] if character == "vak"
               else sorted(glob.glob(os.path.join(MEDIA, "sounds", "companions", "source", f"{character}-*.mp3"))))
    for path in sources:
        x = decode(path)
        peaks, env = onsets(x)
        first = np.argmax(env > env.max() * 0.1)
        peaks = np.unique(np.concatenate([[first], peaks]))
        for i, start in enumerate(peaks):
            end = peaks[i + 1] if i + 1 < len(peaks) else len(x)
            note = x[max(0, start - 64): min(end, start + int(NOTE * SR))]
            f0 = fundamental(note)
            if not f0:
                continue
            # A note that keeps its pitch over time is a clean strike.
            late = fundamental(note, 0.06)
            steady = abs(cents(late, f0)) if late else 1200
            if steady > 25:
                continue
            score = steady - 20 * math.log10(env[start] + 1e-9)
            found.append((score, note, f0, os.path.basename(path)))
    return sorted(found, key=lambda c: c[0])


def repitch(note, f0, target):
    ratio = target / f0
    frac = np.arange(0, len(note) - 1, ratio)
    return np.interp(frac, np.arange(len(note)), note).astype(np.float32)


def shape(note, length=NOTE, fade_s=0.12):
    note = note[: int(length * SR)].copy()
    fade = min(len(note), int(fade_s * SR))
    note[-fade:] *= np.linspace(1, 0, fade, dtype=np.float32)
    note[:32] *= np.linspace(0, 1, 32, dtype=np.float32)
    return note


def render(note, f0, a, b, path):
    first, second = shape(repitch(note, f0, a), GAP, 0.04), shape(repitch(note, f0, b))
    out = np.zeros(int(GAP * SR) + len(second), np.float32)
    out[: len(first)] += first
    out[int(GAP * SR):] += second
    out /= max(1e-6, np.abs(out).max()) / 0.9
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "f32le", "-ar", str(SR), "-ac", "1", "-i", "-",
                    "-af", "loudnorm=I=-20:TP=-3:LRA=7", "-ar", str(SR), "-b:a", "96k", path],
                   input=out.tobytes(), check=True)
    written = decode(path)
    second_at = int(GAP * SR)
    one = written[int(0.015 * SR):int(0.015 * SR) + WINDOW]
    two = written[second_at + int(0.015 * SR):second_at + int(0.015 * SR) + WINDOW]
    m1, m2 = yin(one), yin(two)
    err = (cents(m1, a) if m1 else 9999, cents(m2, b) if m2 else 9999)
    if not (peak_at(one, a) and peak_at(two, b)):
        err = (9999, 9999)
    return (m1, m2), err


def build(character):
    """Tries the cleanest notes in turn until one verifies on both cue notes."""
    a, b = CUES[character]
    path = os.path.join(MEDIA, "sounds", "companions", f"{character}.mp3")
    last = None
    for _, note, f0, source in candidate_notes(character):
        measured, err = render(note, f0, a, b, path)
        last = (source, f0, measured, err)
        if all(abs(e) <= TOLERANCE for e in err):
            break
    if last is None:
        sys.exit(f"{character}: no clean note found")
    return last


def main():
    failed = False
    for character in CUES:
        source, f0, measured, err = build(character)
        ok = all(abs(e) <= TOLERANCE for e in err)
        failed |= not ok
        shown = ", ".join(f"{m:.1f} Hz" if m else "unreadable" for m in measured)
        print(f"{character:5} from {source} (struck at {f0:.1f} Hz) -> {shown}; "
              f"off by {err[0]:+.1f}, {err[1]:+.1f} cents {'ok' if ok else 'FAIL'}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
