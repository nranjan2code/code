# Default workspace audit — 2026-08-30

Status: historical audit of the reviewed tree. Use current source and
`docs/hosting.md` for operator behavior.

## Contract

`vak_config::paths::default_workspace()` is the account-level canonical starting
workspace: `~/vak-home`. It is distinct from the application data directory
(`~/Library/Application Support/vak` on macOS). A user may explicitly choose a
different gateway default in Admin; that choice is persisted under the data
home and is applied after the gateway restarts. Bot and chat workspace choices
remain more specific overrides.

## Findings and fixes

| Surface | Previous risk | Current rule |
| --- | --- | --- |
| launchd/systemd gateway and channel units | `services-sync` captured its caller's cwd | Units are generated with `default_workspace()` and the invoking account's `HOME`. |
| install/update/services-sync | A source checkout could become an always-on chat project | Install creates `~/vak-home`; every sync resolves that same canonical path. |
| `vak serve --gateway` | Gateway Core used the process cwd | Gateway startup resolves the persisted gateway workspace, falling back to `~/vak-home`; non-gateway serve keeps an explicit project cwd. |
| gateway chat dispatch | Bot workspace was displayed/editable but ignored at runtime | Resolution is chat override → inherited bot workspace → gateway default. |
| Admin Overview/Defaults | Health exposed the current process cwd as an unexplained project | Status reports effective workspace, canonical default, known choices, and Defaults & status can save/reset the gateway default. |
| Admin Chats/Bots | Workspace controls were not consistently reflected in dispatch | Existing per-chat and per-bot pickers remain explicit; runtime and status now use the same hierarchy. |

## Install verification performed

The installed 0.11.24 instance was inspected before repair. Its gateway plist
had `WorkingDirectory=/Users/example/Projects/vakcoder`, while the
persisted desktop project and Telegram binding used
`/Users/example/vak-home`. That mismatch explains the screenshots: the
admin health card was showing the gateway Core cwd, while an expanded chat
editor was showing its configured workspace. The generated-unit source and
gateway startup are now independent of the directory from which sync is run.

## Selection hierarchy

```text
chat workspace override
        ↓ (if absent)
bot workspace override (when bot inheritance is enabled)
        ↓ (if absent)
Admin-selected gateway default
        ↓ (if absent/invalid)
~/vak-home
```

Changing a default never rewrites frozen sessions. The next message starts a
new session when the effective workspace changes; existing ledgers remain
append-only and auditable.
