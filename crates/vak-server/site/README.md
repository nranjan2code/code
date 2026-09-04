# The public site at `/`

Four pages — `/`, `/surfaces`, `/security`, `/install` — built from this
directory into `dist/`, which `crates/vak-server/src/site.rs` embeds with
`include_dir!`.

## Change something

```
$EDITOR src/pages/security.html          # or src/styles.css, src/site.js
python3 crates/vak-server/site/build.py  # rebuild dist/
git add crates/vak-server/site           # dist/ is committed
```

`cargo build -p vak-server` refuses to compile against a `dist/` that no
longer matches `src/`, so forgetting the middle step fails loudly on the
next build rather than quietly shipping the previous pages. CI runs
`build.py --check`, and `scripts/release.sh` runs the full rebuild-and-diff.

## Add a page

1. Write `src/pages/<name>.html` — body markup only; the shell is
   `src/layout.html`.
2. Add it to `PAGES` in `build.py` (source, route, nav label, title,
   meta description). That list is also the order of the rail.
3. Add the route to `ROUTES` in `crates/vak-server/src/site.rs`. That table
   is what the router serves *and* what `auth_exempt_path` reads, so the two
   cannot drift apart, and a file appearing in `dist/` never becomes a
   public URL without someone writing it down.
4. Rebuild, and commit `dist/`.

## What lives where

| Path | What it is |
|---|---|
| `src/layout.html` | The shell: head, rail, footer, and the placeholders the builder fills. |
| `src/pages/*.html` | One file per page, body markup only. |
| `src/styles.css` | The whole design system for the site. Inlined into every page. |
| `src/site.js` | Shared behaviour. Inlined into every page. |
| `src/vendor/motion.js` | motion.dev (MIT), pinned and vendored. Emitted as one hashed, shared, deferred file. |
| `dist/` | Generated. Committed, so a clone builds with no Python and no Node. |

## Rules this site has to keep

**It renders before anything else is up.** CSS and `site.js` are inlined,
not linked: the front door must not need a second round trip to show the
product, and a theme that resolves in a second request flashes the wrong
ground. Only `motion.js` is a separate file, because it is 140 KB and
identical on every page.

**It works without JavaScript.** `.reveal` only hides anything once the
inline head script has added `html.js`, so a reader with no script gets the
finished page immediately. `site.js` then upgrades the animation, and every
feature in it is wrapped so that a throw cannot leave content invisible.

**It works without Motion.** `motion.js` is deferred and optional. Without
it the CSS transitions in `styles.css` do the reveals and the scroll
progress rule simply stays empty.

**It discloses nothing.** Every route here answers an unauthenticated
stranger. The only server data a page may read is `/version` — version and
commit, which that endpoint already publishes — and a test enforces exactly
that. `/health` reports provider, model, sandbox and permission mode, and is
not for strangers.

**Product truth only.** No testimonials, customers, benchmarks or pricing
exist, and none may be invented (PRODUCT.md). The gate simulator on
`/security` reproduces the real precedence in
`crates/vak-permission/src/engine.rs`; if that engine changes, the simulator
is wrong until someone changes it too.

## Updating motion.dev

```
python3 crates/vak-server/site/build.py --vendor-motion 13.2.0
python3 crates/vak-server/site/build.py
```

The first command re-fetches `framer-motion`'s browser build and its
licence, keeping the provenance header; the second re-hashes and re-emits.

## Design

The visual world is DESIGN.md ("The Auditor's Desk"), with this surface's
larger type ramp recorded there under *the landing surface's ramp*. The
direction contract for the site is the comment at the top of
`src/pages/index.html`; the strategy is `.impeccable/surfaces/`.
