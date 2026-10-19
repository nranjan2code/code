"""Storage planning inputs: repository facts, never an install inventory."""
import json
import os
import re

import measure as ms

FACTS = [
    ("rotation", "crates/vak-session/src/chain.rs", r"pub const ROTATE_BYTES: u64 = 256 \* 1024;", "Side chains rotate at 256 KiB plus a crossing frame; seal manifests still grow."),
    ("kept", "crates/vak-lifecycle/src/lib.rs", r"Conversation records and", "Conversation records, their objects and run records have no default expiry."),
    ("encrypted", "crates/vak-storage/src/segments.rs", r"let compressed = report.encrypted_frames == 0;", "Sealing encrypted frames copies ciphertext; it does not compress it."),
    ("grace", "crates/vak-core/src/lifecycle.rs", r"const COLLECT_GRACE:.*from_secs\(60 \* 60\);", "Document object collection has a one-hour grace and only releases unreachable objects."),
    ("segments", "crates/vak-core/src/lifecycle.rs", r"const SEAL_AFTER_DAYS: i64 = 30;", "Expiring side chains seal aged segments after 30 days; the newest sealed segment stays."),
    ("scan", "crates/vak-session/src/log.rs", r"pub fn scan\(", "Cold session reads scan segments; bounded model context is not a bounded cold read."),
]


def data(model):
    facts = []
    for key, path, pattern, meaning in FACTS:
        text = open(os.path.join(ms.ROOT, path), encoding="utf-8").read()
        match = re.search(pattern, text)
        if not match:
            model["review"].append(f"storage simulator: {key} missing in {path}")
        else:
            facts.append({"key": key, "file": path, "line": text.count("\n", 0, match.start()) + 1, "meaning": meaning})
    # Read only the shipped default label. This is deliberately not the install's
    # effective retention_label(), nor an implementation of its reconciler.
    text = open(os.path.join(ms.ROOT, "crates/vak-lifecycle/src/lib.rs"), encoding="utf-8").read()
    rules = dict((kind, int(days)) for kind, days in re.findall(r"after\(DataClass::(\w+), (\d+),", text))
    telemetry = re.search(r"delete_after_secs: Some\((\d+) \* DAY\)", text)
    if telemetry:
        rules["Telemetry"] = int(telemetry.group(1))
    required = {"Environment", "Execution", "Telemetry"}
    if not required <= rules.keys():
        model["review"].append("storage simulator: default temporary retention rules missing")
    model["storage"] = {"facts": facts, "default_days": rules,
                        "scope": "Synthetic empty-home forecast; 30-day months. No private data read. No settings changed."}
    return json.dumps(model["storage"], ensure_ascii=False).replace("</", "<\\/")
