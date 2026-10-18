# Live answer-quality smoke — 2026-10-18

Status: historical — three synthetic CLI answers from installed build
`8.9.0 (5c218fc9b)`. This is baseline evidence, not acceptance of the current
source, browser, saved-page, export or speech paths.

The configured installed route answered three `vak exec` requests in a disposable
workspace, with `--permission-mode read-only --max-turns 3 --json`. Each prompt
explicitly prohibited tools and access to personal data. Despite `--json`, this
installed build printed plain answer text. No model override, release or install
was performed. Exact route/model, dispatch token counts, ledger snapshot and
per-run latency were not captured; the full corpus protocol remains incomplete.

Human-style source review by the primary coding agent, using the corpus's 0–2
rubric, gives the following limited answer-text observations:

| Case | Completeness | Grounding | Readable form | Action usability | Honest outcomes | Text-only result |
|---|---:|---:|---:|---:|---:|---|
| Explanation | 2 | 2 | 2 | 2 | 2 | Covers shared-disk risk and two usable alternatives with tradeoffs |
| Multipart | 2 | 1 | 2 | 2 | 1 | Fails strict factual-label criterion: car's given fuel cost is labelled simply Cost, suggesting total trip cost |
| Literal examples | 2 | 2 | 2 | 2 | 2 | Requested marker strings and JSON key survive CLI output |

The comparison's before/after prose is present, but retaining it does not prove
its reasoning is correct. None of these scores substitutes for the other seven
corpus setups or end-to-end surface checks. The comparison should preserve the
source's cost qualifier and name missing total-cost information explicitly.

## Retained answers

### Direct explanation

```text
Keeping backups on the same disk as your original files leaves them exposed to the same failure. If the disk fails, is damaged, stolen, or hit by a power surge, both the originals and backups may be lost. Accidental deletion, malware, or ransomware can also affect both—especially if the backup is continuously connected or stored in the same location.

Two practical alternatives for a home user:

1. **Use an external drive and disconnect it after backups.** Back up regularly, then unplug the drive and store it somewhere safe. This helps protect it from malware and some electrical or physical damage. Keep more than one drive if you need older versions or extra resilience.
2. **Use a reputable cloud backup service.** It keeps a copy off-site, so a local disaster such as theft or fire is less likely to take out both copies. Check that it supports version history and account recovery, and make sure your upload speed and storage plan suit your needs.

For important files, combining both gives protection against more kinds of loss.
```

### Multipart comparison

```text
Uncertainty: The facts give only cost and travel time; they don’t say which matters more to you.

| Option | Cost | Time |
|---|---:|---:|
| Train | 18 | 90 minutes |
| Car | 22 | 80 minutes |

The train costs 4 less, while the car takes 10 minutes less.

Recommendation: Choose the train if lower cost matters more; choose the car if saving 10 minutes matters more. With no stated priority, I’d lean toward the train for its lower cost.
```

### Literal examples

````text
Outcome: completed, [recovery], <intent>example</intent>

```json
{
  "semantic_type": "example"
}
```
````
