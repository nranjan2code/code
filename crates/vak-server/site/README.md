# Vakyartha public site

Fifteen static pages, built here and embedded by `crates/vak-server/src/site.rs`.
The navigation uses plain labels. `/doctor` is reserved for the authenticated
runtime API; the public introduction lives at `/meet-doctor`. Only the standalone
Vercel export redirects the old website URL to that introduction.

| Route | Purpose |
| --- | --- |
| `/` | Character scenes and expandable everyday, work and learning examples |
| `/outcomes` | All eight companions with example requests and possible results |
| `/email-calendar` | Selected mail, calendar planning, reviewed actions and useful routines |
| `/headless-cloud` | Own-server hosting, cloud deployment, and remote access |
| `/tour` | Interactive request/result illustrations and a short walkthrough |
| `/security` | Illustrated guide to access, approvals, data, connected chats, interruption and evidence |
| `/install` | Public source build instructions and access to an existing installation |
| `/architecture` | Illustrated architecture, ledger, context, voice, FinOps, admin and deployment |
| `/meet-doctor` | Everyday diagnostics and supported repairs, told in three illustrated scenes |
| `/surfaces` | Desktop, browser, connected chats and terminal |
| `/wallpapers` | All 16 desktop and mobile wallpaper downloads, without sign-in |
| `/vak` | The name and Songbird identity |
| `/terms` | Terms of use, software license, warranty and liability limits |
| `/privacy` | Public website and self-hosted software data handling |

## Edit and build

```sh
python3 crates/vak-server/site/build.py
python3 crates/vak-server/site/build.py --check
```

Edit `src/pages/*.html`, `src/layout.html`, `src/styles.css` or `src/site.js`,
then regenerate `dist/`. The generated bundle is committed. The Rust build
checks its source manifest; it must not embed a stale bundle.

## Publish the static site on Vercel

The public domain uses Vercel's `www` project. Build a separate export from
the same site and deploy that directory with the desktop CLI:

```sh
python3 crates/vak-server/site/build.py --check
python3 crates/vak-server/site/build_vercel.py /tmp/vakyartha-site-export
vercel deploy /tmp/vakyartha-site-export --project www
```

Inspect the preview, then promote its deployment ID with `vercel promote ID`.
Use a fresh output directory for each export. The export selects Vercel's
static framework, copies the brand assets, provides `/version`, and changes
the links that only work on a running Vak server. The embedded site is not
changed by this export.

The builder copies WebP, JPEG and PNG files recursively from `src/assets/` to `/site/`. CSS and the shared script
are inlined. The optional, pinned Motion library is emitted under its content
hash. Only the walkthrough's example transition uses Motion; content does not
wait for it, and reduced motion disables the transition.

To add a page, update `PAGES` in `build.py` and `ROUTES` in `src/site.rs`.
`NAV_ROUTES` selects the primary navigation. A file appearing in `dist/` never
implicitly becomes a public page.

## Your control page

`/security` explains the runtime’s actual boundaries in everyday language, with
three Vakyartha-only illustrations and a clearly labelled example review.
Important limitations remain visible: full access, connected-service data flow,
partial results, spending estimates and Trash versus erasure. Deeper mechanics
link to the source. Evidence: `docs/assets/public-site-security-2026/`.

## Architecture page

`/architecture` uses Vakyartha as its only character. Generated transparent
illustrations introduce the core, request journey, workspace pool, ledger, context and expenditure. Doctor has three dedicated scenes.
Readable HTML labels and explanations carry the technical meaning. Source links
expose the implementation and distinguish shipped voice and self-hosting from
planned realtime and cloud-remote work. Prompts and originals are in
`docs/brand/library/public-site-scenes/architecture/`.

## Direction

The follow-through content pass adds ongoing-work storytelling, commitments,
memory and control in everyday language. Vakyartha and the seven companions
retain distinct personalities from the canonical `agentGlyph.ts` registry;
their personalities are not fixed job roles. A new transparent continuity
scene accompanies the existing three activity scenes. Review evidence and
preview status: `docs/assets/public-site-followthrough-2026/README.md`.

The owner requested a complete visual rethink on 2026-09-27: minimal text,
modern and peppy across generations, all eight characters actively doing
things, no paper treatment, no repeated marketing-card grid, no em dashes.
The public site therefore uses crisp white and charcoal, system sans type,
large transparent character scenes and varied compositions.
Navigation actions use regular-weight text, a small arrow and a fine underline,
with a 44px minimum touch target. Example review buttons share the same
quiet treatment. No action uses a solid indigo fill. This is an
explicit site-specific departure from the old Auditor's Desk and cream-paper
marketing treatment. It does not change the client's design tokens.

Official Songbird and wordmark exports remain the identity. The three new
activity scenes use the existing Dimensional character identities. Originals,
reference list and generation prompts are in
`docs/brand/library/public-site-scenes/`. The WebP exports total about 750 KB;
only the hero loads eagerly. No remote fonts or image services are required.

## Contracts

- Public pages read only `/version`, and show it inside Build details.
  Never fetch authenticated machine state or `/health`.
- Examples and review interactions are illustrations, not live AI runs or
  real file mutations. Characters are choices of companion, not fixed roles.
- No invented testimonials, pricing, customer counts, benchmarks or downloads.
  Setup states the current public-source and local-build requirements.
- Native request disclosures and navigation work without JavaScript. The
  walkthrough keeps its first complete example and links to the examples page.
  Script-only controls start hidden. Theme storage failure is harmless.
- Keep keyboard operation, visible focus, reduced motion, readable text and
  both themes. Body copy is 16px and no text is below 12px.
- Do not edit generated brand masters or `dist/` by hand.

## Verification

The site has been checked in a browser at 1440 × 900 and 390 × 844 in both
light and dark. Screenshots and the review record are under
`docs/assets/public-site-refresh-2026/`. The architecture pass is recorded in
`docs/assets/public-site-architecture-2026/`.

```sh
node --check crates/vak-server/site/src/site.js
python3 crates/vak-server/site/build.py --check
cargo test -p vak-server --lib site::tests
cargo test -p vak-server --test server_ext public_doctor_page_preserves_authenticated_doctor_api
```

The focused Rust tests cover every public page, trailing slashes, referenced
`/site/` assets and the public-data fetch boundary.

Wallpaper sources live in `docs/brand/library/wallpapers/`. Regenerate public
assets with `npm --prefix scripts/brand run wallpapers`, then rebuild this site.
