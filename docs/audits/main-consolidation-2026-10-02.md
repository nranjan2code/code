# Main consolidation — 2026-10-02

Status: integrated and validated on `main`; push and checkout cleanup in progress.

The maintainer requested consolidation of all pending work into `main`, a push
to `origin`, and removal of the other local branches and worktrees. The primary
checkout is `code`. Integration starts from `dd2ea6633` without rewriting the
published history or changing the workspace version.

## Sources and disposition

| Source | Disposition |
|---|---|
| `social-feature` working tree and its three stashes | Preserve the newest LinkedIn native PKCE identity flow, Settings controls, tests, prompts, registry metadata and presentation seed documentation. Earlier social package and adapter work is already on main. Rebuild the client bundles from the combined sources. |
| `codex/budget-gaps` | All committed work through `dd2ea6633` is already on main. Include the two untracked channel-delivery and intent/commitment audits, with their reproduction sources and results. The defects those audits identify remain explicitly open. |
| Detached `vak-escape` and `cool-lumiere-72b5da` worktrees; the Claude branch | Their tips are ancestors of main and their working trees are clean. No unique source changes remain to import. |
| Office stash `3b3b0423b` | All seven code/test/document patches are already applied. The system prompt carries the equivalent document limits in newer, shorter wording. Retain current code. |
| Canvas stash `d3463972d` | The current Canvas stack, per-conversation state, viewer registry and subject identity include and extend this work. Do not restore duplicate subject functions or remove current tests. |
| Website stash `484fb59a2` | Recover the hero animation and pointer behavior onto the current complete artwork, with reduced-motion handling. Preserve its unstyled alternative scene fragment under `docs/research/site-hero-prototype-2026-10-02/` as historical source. Rebuild the website. |
| Dependabot `d1c97f63f` | Include the DOMPurify 3.4.16 lockfile update and rebuild the client. |
| Previously advertised Dependabot `832410a56` | Include the fast-uri 3.1.8 video lockfile update. Its remote branch disappeared during the inventory fetch; the commit is preserved in the recovery bundle. |
| Six archived Codex worktree snapshots | Two tips are already ancestors, two have patch-equivalent changes on main, and the remaining brand/provider snapshots are incorporated in newer source. All brand assets are present; obsolete generated bundle filenames and a dependency-cache symlink are not restored. |
| Historical ledger-lock wait archive | Superseded by `cc9af8b13`'s explicit lock release and safe process-group setup. Preserve the shipped implementation rather than introducing the earlier polling workaround. |

## Verification and recovery

The initial inventory, each worktree's binary diff and untracked files, all six
stash patches, Git refs and a verified full-history bundle were saved outside
the checkout in `consolidation-backup-2026-10-02`. Recovery data is retained
after branch and worktree cleanup.

The full test gate initially rejected two presentation-only `weather` labels
in the built-in seeds. Its existing exceptions already allow the same card
vocabulary in the emitting and receiving registries. The exception now also
allows only the two exact seed declarations, with a regression test that
continues to reject topic-based routing in that file and in agent code.

The website was opened locally in the browser: its hero image loaded, the
animation and pointer setup were present, and no console error was observed.
With reduced motion emulated, the animation and transform were disabled and
the pointer handler was absent. Browser emulation was reset after the check.

Repository validation passed on the integrated tree: `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, `scripts/check-version.sh`,
`python3 scripts/check_doc_paths.py`, the client UI production build, the
website build, and the video package typecheck. The DOMPurify package audit
reported zero vulnerabilities. Dimensional visual-pack styling from the older
captured source was verified in both current client and admin styles.

Live social provider approval, API eligibility and the open defects in the
imported audits are not claimed as completed by this repository consolidation.
