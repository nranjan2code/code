"""Build current library, run audit expectations, and retain evidence beside this file.

Expected to return nonzero while the audited defects are present. No network sends,
credentials, server, or real data home are used. The executable lives in /tmp.
"""
from pathlib import Path
import subprocess
import tempfile

here = Path(__file__).resolve().parent
root = here.parents[2]
subprocess.run(["cargo", "build", "-p", "vak-delivery", "--lib"], cwd=root, check=True)
with tempfile.TemporaryDirectory(prefix="vak-channel-audit-") as temp:
    binary = Path(temp) / "probes"
    subprocess.run([
        "rustc", "--edition=2024", "--test", str(here / "probe.rs"),
        "-L", f"dependency={root / 'target/debug/deps'}",
        "--extern", f"vak_delivery={root / 'target/debug/libvak_delivery.rlib'}",
        "-o", str(binary),
    ], cwd=root, check=True)
    result = subprocess.run([str(binary), "--nocapture", "--test-threads=1"],
                            cwd=root, text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT)
    (here / "probe-results.txt").write_text(result.stdout)
    print(result.stdout)
    raise SystemExit(result.returncode)
