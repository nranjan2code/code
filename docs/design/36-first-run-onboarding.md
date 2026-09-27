First-Run Onboarding
====================

Status: historical v3.0.24 wizard contract. For the current setup and install
behavior use `docs/release-and-install.md` and `vak setup --help`.

## Overview

The first-run onboarding wizard (`vak setup`) seeds a new workspace with
default skills, capabilities, and a prompt seed. This document records that
release's surface contract; `docs/design/46-stabilization-install-and-onboarding.md`
is a historical proposal, not the current operator runbook.

## Entry Points

| Surface | How onboarding triggers |
|---|---|
| CLI | `vak exec "first prompt"` on an unconfigured workspace |
| Server | First request to `/sessions` without a `.vak/config.toml` |
| Desktop | `AppState::new` with `trust_project = false` and no config |

## Wizard Steps (P0)

1. **Provider selection**: Prompt for API key (or Ollama endpoint). Keys are
   written to the project secret scope, resolved through
   `vak_config::credentials` (docs/design/44-shared-config.md, "Secrets
   Chain") — never a plaintext file — and read back via `vak_config::get_var`.
2. **Posture selection**: Choose `read-only`, `workspace-write`, or
   `full-access` permission mode.
3. **Skill seeding**: Populate `~/.vak/skills/` with the curated skill catalog
   (domain proposals from `docs/design/26-learning.md`).
4. **Trust confirmation**: Ask the operator to confirm the workspace root.
   Until confirmed, privileged config keys are stripped (rule 27).

## Non-Interactive Mode

`vak setup --non-interactive` requires env vars:
- `VAK_SETUP_PROVIDER` (e.g. `openai`, `anthropic`, `ollama`)
- `VAK_SETUP_MODEL` (e.g. `gpt-4.1`, `llama3`)
- `VAK_SETUP_POSTURE` (`read-only` / `workspace-write` / `full-access`)
- `VAK_SETUP_SEED` (0 or 1; seeds default skills when 1)

Returns exit code 2 when a choice is missing and refuses to fall back to
a default provider key.

## Security

- The wizard never reads the operator's real secret scope — it writes a fresh one.
- Provider keys are never echoed, logged, or stored in `.vak/config.toml`.
- The `--trust` flag must be passed explicitly by the operator; an untrusted
  first-run configuration cannot grant itself execution power.
