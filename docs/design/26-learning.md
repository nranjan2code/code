# 26 — Learning loop: memory write-path + skill proposals

Closes pillars M2 (memory write-path) and G5 (self-improving skills behind
review). Hermes differentiates itself with a closed loop — experience becomes
durable knowledge. We implement the loop with two model-invoked tools and one
human gate:

```
run completes ── model decides something is worth keeping
   │
   ├─ remember(note, kind?, tag?)        → MEMORY.md block (immediate)
   │      recalled by session_search, ranked above raw transcripts
   │
   └─ propose_skill(name, description, instructions)
          → review queue (<home>/skill-proposals/)
                 │  human promotes (HTTP/CLI/desktop later)
                 ▼
          <home>/skills/<name>/SKILL.md   ← discovered for EVERY future run
```

## Principles

1. **Model proposes; only humans promote skills.** A proposed skill can
   redirect arbitrary future runs — it crosses the trust boundary and must
   pass an explicit gate (invariant: unattended surfaces fail closed;
   promotion is never automatic, never time-based).
2. **Notes are cheap, visible, revocable.** MEMORY.md is plain markdown the
   user can edit or prune; every block carries provenance (timestamp, kind,
   tag, originating session id).
3. **Model-visible ⇒ logged.** Both tools are ordinary tools: the call args
   and confirmations land on the ledger as tool_use/tool_result entries.
   Promoted skills enter discovery, whose names+descriptions are captured in
   every later FrozenContract.
4. **No ambient cost.** Nothing is injected into prompts by this feature;
   recall flows exclusively through `session_search`, which now also ranks
   memory blocks (with an outranking bonus) against transcripts.

## Storage

```
<home>/memory/<hash_cwd>/MEMORY.md        # notes, per workspace, plain md
<home>/skill-proposals/<hash_cwd>/<id>.md # pending proposals, SKILL.md shape
<home>/skills/<name>/SKILL.md             # promoted (user-level discovery)
```

MEMORY.md block grammar (tolerant to manual edits):

```
## <RFC3339> [<kind>] tag=<tag> session=<session-id>
<free-form markdown lines>
```

Parser: split on `## ` headings; `[kind]` optional (defaults `note`);
unknown `key=value` tokens preserved; malformed headings become ordinary
text of the preceding block — hand-edits never lose data.

## Tools

| Tool | Args | Effect | Permission |
|---|---|---|---|
| `remember` | `note` (req), `kind` (fact\|decision\|preference\|reference), `tag` | append block | Allowed in WorkspaceWrite+ (journaling into vak's own per-workspace store, like session ledgers themselves); denied in ReadOnly |
| `propose_skill` | `name`, `description`, `instructions` | queue file | Same |

The permission engine classifies both explicitly so they do not fall into
the generic unknown-tool Ask path — otherwise every phone-side `remember`
would stall on a forwarded gate.

## Recall

`vak_session::search_extended(..., extras: &[ExternalDoc])` scores memory
blocks with the same scorer plus a bonus that places them above equally
relevant transcript lines; hits surface as `role: "memory"` with the block's
tag/kind in the snippet prefix. `session_search` and `GET /search` both feed
parsed MEMORY.md entries in.

## Promotion

- `GET /skills/proposals` — queue listing (id, name, description, age).
- `POST /skills/proposals/{id}/promote` — installs to
  `<home>/skills/<name>/SKILL.md`; refuses silent overwrite of an existing
  skill name.
- `POST /skills/proposals/{id}/reject` — deletes the pending file.
- CLI mirrors: `vak skills-review list|promote|reject`, `vak memory`.

Provenance footer (HTML comment) survives inside SKILL.md harmlessly:
`<!-- proposed-by: <session-id> at <ts> -->`.

## Configuration

```toml
[memory]
write_enabled    = true   # remember tool available to runs
skill_proposals  = true   # propose_skill available; queue always reviewable
```

Local-only and workspace-scoped, hence not privileged — but ReadOnly mode
still denies both tools outright.

## Phases

| Phase | Delivers | Exit criterion |
|---|---|---|
| **L0 (this)** | memory store + parser, search boost, both tools, engine classification, promotion endpoints + CLI, config flags | unit roundtrips incl. hand-edited files; search outranks transcripts; e2e: agent remembers mid-run, a later run recalls it citing memory; promote→discovery loop closes; fmt/clippy/tests green |
| **L1 ✅** | Post-run reflection (`[memory] reflection = true`): auxiliary model call after clean gateway completions proposes ≤2 notes + optional skill draft; Jaccard dedup (≥0.55) against existing MEMORY.md bounds growth; skill drafts land in the same human-gated review queue; detached best-effort — never affects the reply | parser/dedup/e2e tests: near-duplicate second run writes 0 notes; invalid entries don't consume slots |
| **L2 ✅** | Desktop Settings ▸ Learning page: proposal queue with Promote/Reject + memory-note viewer (tag/kind/session provenance), 8 s polling while open; TUI `/services` ships alongside in the same ops pass | tsc/vite clean; hermetic gateway fixtures keep scripted flows deterministic |
