# Presentation Delivery Audit — Final Output Verification

Status: completed
Date: 2026-09-10
Related design docs: docs/design/30-output-engineering.md, docs/design/30-render-architecture.md

## Overview

Three test suites in `crates/vak-delivery/tests/` comprehensively verify the
presentation delivery pipeline. Together they cover 20,922 scenarios, all
passing.

| Suite file | Scenarios | Test functions | Focus |
|---|---|---|---|
| `presentation_audit.rs` | 922 | 18 | Full content assertions on specific output patterns |
| `audit_sweep_10k.rs` | 10,000 | 20 | Non-panic sweep across 10 adversarial + 10 defensive categories |
| `audit_final_output_10k.rs` | 10,000 | 24 | **Content-correctness** and **structural-integrity** assertions on final rendered output across all 6 surfaces |

## Surfaces and Markups

The delivery pipeline maps each surface to a markup type:

| Surface | Markup | Description |
|---|---|---|
| `telegram` | `TelegramHtml` | Telegram-native HTML (`<b>`, `<pre>`, `<tg-spoiler>`, `<blockquote>`, escaped `<`/`>`/`&`) |
| `discord` | `DiscordMarkdown` | Discord-flavored markdown (`# ` → `<b>` for H1 only, spoilers as `||text||`, HR as `─`) |
| `slack` | `SlackMrkdwn` | Slack mrkdwn (`# ` → `*text*`, HR as `─`) |
| `desktop` | `Markdown` | Standard CommonMark markdown |
| `terminal` | `Plain` | Plain text (strips formatting, code blocks yield raw content) |
| `admin` | `Plain` | Plain text |

## Pipeline

```
DeliveryJob → render() → DeliveryPacket
```

`render()` calls `render_content()` which:
1. Projects the answer via `channel_projection()` (handles structured fence projection per markup).
2. Renders markdown to the target markup via `render_answer()` / `render_text()`.
3. Builds coverage entries from projected answer blocks.
4. Assembles chunks and diagnostics.

The `fallback_markdown` field always preserves the original source markdown verbatim.

## `audit_final_output_10k.rs` — Red Team (5,000 scenarios)

### content_headings_output (500)
- Verifies headings render correctly per surface.
- **Telegram**: `#` headings → `<b>` (all heading levels).
- **Discord**: H1 (`# `) → `<b>`; H2+ (`## `+) kept as-is.
- **Desktop/Admin**: heading text preserved (contains `Title`/`Subtitle`/`H` or `#`).

### content_code_blocks_output (500)
- **Telegram**: ` ``` ` code blocks → `<pre>`/`<code>`.
- **Desktop/Discord/Slack**: ` ``` ` fence preserved or code content present.
- **Terminal/Admin**: Code fences stripped; content (`fn`, `code`, `removed`, `print`) preserved; empty code blocks (` ``` ``` ``` `) produce empty text.

### content_lists_output (500)
- List items preserved across all surfaces.
- Task list markers (`☐`/`☑`) on Telegram and Slack; `[ ]`/`[x]` preserved on Desktop/Discord/Terminal/Admin.

### content_emphasis_output (500)
- **Telegram**: `**bold**` → `<b>`; `~~strike~~` → `<s>`.
- Other surfaces: emphasis markers preserved or content present.
- Discord does NOT convert `**bold**` to `<b>` (keeps `**bold**`).

### content_tables_output (500)
- Table data cell values (1, val1, single, A) appear in output across all surfaces.

### content_links_output (500)
- Link text (label, text, alt) or URLs preserved across all surfaces.
- Telegram: `[text](url)` → `<a href="url">text</a>`.
- Discord: `text (<url>)`.
- Slack: `<url|text>`.

### content_quotes_output (500)
- **Telegram**: `<blockquote>` tag or content present.
- Other surfaces: non-empty for non-empty input.

### content_html_escaping_output (500)
- **Telegram HTML**: `<script>`, `<iframe>`, `<img>`, `<div>` tags must NOT appear unescaped in output.
- All surfaces render without panic for raw HTML input.

### content_spoilers_output (500)
- **Telegram**: Single-line `||spoiler||` → `<tg-spoiler>hidden</tg-spoiler>`.
- Multi-line spoilers (`||multi\nline\nspoiler||`) may not be recognized (spoiler markers on separate lines).
- Spoiler content text present in output.

### content_mixed_structures_output (500)
- Complex documents mixing headings, bold, lists, code blocks, quotes, tables, spoilers.
- Verifies: fallback preservation, schema version, unique coverage block IDs.

### content_special_characters_output (500)
- Emoji, CJK, math symbols, RTL override, zero-width, BOM, special chars (`< > &`).
- **Telegram**: raw `<script>` must not appear in output for `<`-containing input.
- Non-empty output for non-empty input on all surfaces.

### content_horizontal_rules_output (500)
- `---`, `***`, `___` → `─` (box-drawing horizontal line) on Telegram, Discord, Slack.
- Plain surfaces: `---` preserved as-is.
- Fallback always preserved.

### content_empty_and_whitespace_output (500)
- Empty strings, spaces, tabs, newlines, BOM, zero-width space.
- Fallback preserved verbatim.
- Empty coverage for whitespace-only input.

### content_paragraphs_and_linebreaks_output (500)
- Single/double/triple newlines, trailing/leading newlines.
- Fallback preserved. Content values (`one`, `two`) present in output.

### content_nested_constructs_output (500)
- Nested bold/italic, code within headings, spoilers with emphasis, code in tables.
- Non-empty output; fallback preserved.

## `audit_final_output_10k.rs` — Blue Team (5,000 scenarios)

### structural_packet_fields (500)
- `schema_version == DELIVERY_SCHEMA_VERSION` (2).
- `job_id == "test-job"`.
- `surface` matches rendering surface.
- `kind == DeliveryKind::Assistant`.
- `target == "<surface>:one"`.

### structural_fallback_preservation (500)
- `packet.fallback_markdown == source` on all 6 surfaces for 12 template variants.

### structural_coverage_integrity (500)
- Coverage block IDs are unique (via `BTreeSet` deduplication check).
- No empty block IDs.
- Non-empty coverage for non-empty input.

### structural_chunking_validity (500)
- All chunks are non-empty.

### structural_payload_types (500)
- `Markup::Json` → `DeliveryPayload::Structured`.
- All other markups → `DeliveryPayload::Text`.

### structural_serialization_roundtrip (500)
- Full `serde_json::to_string` → `serde_json::from_str` round-trip.
- All fields compared: schema_version, job_id, surface, kind, target, payload,
  fallback_markdown, chunks, coverage, actions, diagnostics.

### structural_max_chars_truncation (500)
- `max_chars = Some(100)` enforced.
- All chunks non-empty even under truncation.

### structural_cross_surface_consistency (500)
- Same `fallback_markdown` across all 6 surfaces.
- Same `schema_version` across all surfaces.
- Same `kind` across all surfaces.
- Same `job_id` across all surfaces.

### structural_content_kind_matching (500)
- `kind == DeliveryKind::Assistant` on all surfaces.
- `target` format correct.
- `job_id` preserved.

## `presentation_audit.rs` — 922 scenarios (existing)

18 test functions covering schema consistency, payload types, fallback chains,
surface-specific markup, HTML escaping, spoiler handling, task lists, code
fences, headings, links, nested constructs, serialization, diagnostics,
outbox state machine, worker protocol, signal extraction, skill validation,
and template revision security. See CHANGELOG 3.0.55 and earlier.

## `audit_sweep_10k.rs` — 10,000 scenarios (existing)

20 test functions × 500 scenarios using `catch_unwind` for non-panic verification
across adversarial (XSS, malformed tables, code fence abuse, unicode, extreme
inputs, link injection, nested markdown, format strings, null/empty) and
defensive (error paths, HTML sanitization, determinism, coverage accounting,
profile validation, template security, outbox state, signal extraction, skill
validation, worker protocol) categories. See CHANGELOG 3.0.55 and earlier.

## Source fixes applied (all committed)

| File | Fix |
|---|---|
| `src/skills.rs` | `signal_text_hit()` uses `contains` for needles with non-word-boundary chars (`https://`, `source:`, `°c`) |
| `src/lib.rs` | `TemplateRegistry::upsert()` returns `Err(InvalidTemplate)` when `template.revision < existing.revision` |
| `src/telegram.rs` | `is_spoiler()`, `strip_task_item()`, task list handling before regular list check |
| `src/discord.rs` | `is_spoiler()`, `#` → `<b>` heading conversion, `strip_task_item()`, task list check before heading check |
| `src/slack.rs` | `is_spoiler()`, `strip_task_item()`, task list check before regular list check |

## Running the tests

```sh
cargo test -p vak-delivery --test audit_final_output_10k
cargo test -p vak-delivery --test audit_sweep_10k
cargo test -p vak-delivery --test presentation_audit
```
