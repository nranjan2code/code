---
name: vakcoder
description: A coding agent you can inspect, constrain, and extend.
colors:
  bg: "#171714"
  surface: "#1c1c19"
  surface-raised: "#22221f"
  surface-hover: "#292925"
  surface-active: "#30302b"
  sidebar: "#1f1f1c"
  border: "#34342f"
  border-soft: "#2a2a26"
  text: "#eeeae2"
  text-soft: "#c4c0b8"
  muted: "#918e86"
  faint: "#68665f"
  burnt-terracotta: "#df795f"
  burnt-terracotta-bright: "#ee9278"
  burnt-terracotta-wash: "rgba(223, 121, 95, 0.13)"
  green: "#73a982"
  yellow: "#d4a85d"
  red: "#d86f72"
  blue: "#7c9fc9"
typography:
  headline:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Inter', 'Segoe UI', sans-serif"
    fontSize: "22px"
    fontWeight: 620
    lineHeight: 1.2
    letterSpacing: "-0.02em"
  title:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Inter', 'Segoe UI', sans-serif"
    fontSize: "15px"
    fontWeight: 620
    lineHeight: 1.3
    letterSpacing: "-0.01em"
  body:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Inter', 'Segoe UI', sans-serif"
    fontSize: "13px"
    fontWeight: 500
    lineHeight: 1.5
    letterSpacing: "normal"
  label:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Inter', 'Segoe UI', sans-serif"
    fontSize: "11px"
    fontWeight: 550
    lineHeight: 1.4
    letterSpacing: "normal"
  mono:
    fontFamily: "'SFMono-Regular', 'SF Mono', ui-monospace, Menlo, Consolas, monospace"
    fontSize: "13px"
    fontWeight: 500
    lineHeight: 1.5
    letterSpacing: "normal"
rounded:
  sm: "7px"
  md: "10px"
  lg: "14px"
  pill: "999px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "22px"
components:
  button-primary:
    backgroundColor: "{colors.burnt-terracotta}"
    textColor: "#201713"
    rounded: "{rounded.sm}"
    padding: "5px 11px"
  button-primary-hover:
    backgroundColor: "{colors.burnt-terracotta-bright}"
  button-secondary:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.text-soft}"
    rounded: "{rounded.sm}"
    padding: "5px 11px"
  button-secondary-hover:
    backgroundColor: "{colors.surface-hover}"
    textColor: "{colors.text}"
  chip:
    backgroundColor: "transparent"
    textColor: "{colors.muted}"
    rounded: "{rounded.pill}"
    padding: "2px 8px"
  chip-active:
    backgroundColor: "{colors.surface-active}"
    textColor: "{colors.text}"
    rounded: "{rounded.pill}"
  card:
    backgroundColor: "{colors.surface}"
    rounded: "{rounded.md}"
    padding: "13px"
  input:
    backgroundColor: "{colors.surface-active}"
    textColor: "{colors.text}"
    rounded: "{rounded.sm}"
    padding: "5px 10px"
---

# Design System: vakcoder

## Overview

**Creative North Star: "The Auditor's Desk"**

vakcoder's UI is a quiet, focused workspace for someone reviewing serious work — the desktop client and admin portal both read as instruments for a practiced hand, not showrooms for a brand. Density is high but never cramped: an odd, deliberately calibrated type scale (font-weights like 540, 560, 580, 620 rather than round hundreds) and tight, consistent spacing give every control the feel of having been tuned, not eyeballed. The palette stays almost entirely neutral — warm near-blacks and soft off-whites — so that the single accent, a burnt terracotta, reads as a genuine signal every time it appears: a brand mark, a primary action, an active nav item, a running session's pulse dot. Depth is conveyed through tonal layering (background → surface → surface-raised → surface-hover) rather than shadow; shadow is reserved for things that must visibly float above the layout.

The system deliberately rejects the generic flat-blue enterprise SaaS look, and it rejects skeuomorphism, heavy gradients, or glassmorphic chrome. Nothing is decorative. Every visual choice — a border color, a radius, a font-weight — is there to make dense, high-stakes information (diffs, approvals, receipts, permission state) scannable at speed without shouting.

**Key Characteristics:**
- Dark-first, warm near-black surfaces with tonal (not shadow-based) depth at rest
- One accent color, spent sparingly, so it always means something
- A dense, odd-weight type scale tuned in single-digit increments, not round steps
- Flat by default; box-shadow appears only on things that float (modals, toasts, popovers)
- SF Mono / system-mono for anything code- or receipt-shaped; system sans everywhere else

## Colors

The palette is a warm, near-monochrome dark scale (off-black through warm off-white) with one accent spent deliberately, plus a small fixed set of status colors that never shift role.

### Primary
- **Burnt Terracotta** (`#df795f`): the sole accent. Brand mark, primary buttons, active nav/session state, focus rings, running-session pulse. Its rarity — reserved for "this is active" or "this is the primary action" — is the point.
- **Burnt Terracotta Bright** (`#ee9278`): hover/pressed state of the accent, and icon-button "on" state.
- **Burnt Terracotta Wash** (`rgba(223, 121, 95, 0.13)`): soft background fill behind an active accent element (e.g. the brand mark tile).

### Neutral
- **Void** (`#171714`, `--bg`): app background, the deepest layer.
- **Surface** (`#1c1c19`, `--surface`): base panel/card layer, one step up from void.
- **Surface Raised** (`#22221f`, `--surface-raised`): buttons, composer, modals — the layer things sit "on."
- **Surface Hover** (`#292925`) / **Surface Active** (`#30302b`): interactive-state layers, one and two steps brighter than raised.
- **Sidebar** (`#1f1f1c`): the sidebar's own distinct panel tone, between surface and raised.
- **Border** (`#34342f`) / **Border Soft** (`#2a2a26`): default and quiet dividers.
- **Warm Paper** (`#eeeae2`, `--text`): primary text, warm off-white rather than pure white.
- **Text Soft** (`#c4c0b8`): secondary text.
- **Muted** (`#918e86`): tertiary/label text, icon default color.
- **Faint** (`#68665f`): placeholders, timestamps, the quietest text on the page.

### Status (fixed roles, used only for state — never decorative)
- **Green** (`#73a982`): success, running/active indicators.
- **Yellow** (`#d4a85d`): pending approval, warning, tool-call-in-flight.
- **Red** (`#d86f72`): danger, destructive actions, errors.
- **Blue** (`#7c9fc9`): informational accent; also doubles as the accent color in the alternate `data-theme="dark"` palette.

### Named Rules
**The One Accent Rule.** Burnt Terracotta is the only color used to mean "this is the primary thing" or "this is active." It never doubles as decoration; if a new element needs emphasis, reach for a status color (if it's stateful) or a tonal layer (if it's structural) before reaching for the accent again.

## Typography

**Body/UI Font:** `-apple-system, BlinkMacSystemFont, "SF Pro Text", "Inter", "Segoe UI", sans-serif` — the system sans stack, used for all UI chrome.
**Mono Font:** `"SFMono-Regular", "SF Mono", ui-monospace, Menlo, Consolas, monospace` — used for code blocks, receipts, terminal, and any raw data.

**Character:** A working developer-tool voice — the system sans stack keeps it native and fast-rendering rather than making a typographic statement, and precision comes instead from a tightly tuned weight and size scale (odd values like 540, 560, 580, 620; 10.5px, 11.5px, 12.5px) that reads as calibrated rather than templated.

### Hierarchy
- **Headline** (620, 22px, 1.2 line-height, -0.02em): empty-state headers (e.g. chat empty state).
- **Title** (620, 13.5–15px, 1.3, -0.01em): workspace title, modal titles.
- **Section Label** (600, 12px): sidebar section titles, nav group headers.
- **Body** (500, 13px, 1.5): default UI text, messages, descriptions.
- **Item Label** (540–580, 12–12.5px): sidebar items, nav items, form labels.
- **Micro Label** (500–550, 10–11.5px): timestamps, hints, badges, hotkey glyphs, tooltips — always paired with `--muted` or `--faint` color, never `--text`.

### Named Rules
**The Odd-Weight Rule.** Font-weights are tuned in single-digit steps (500, 540, 550, 560, 570, 580, 600, 620, 650) rather than snapped to round hundreds. Treat existing weights as calibrated values to reuse, not round to the nearest 100.

## Layout

A three-region CSS grid app shell: `sidebar` (fixed ~278px, collapsible to 0) / `main` (flexible, `minmax(420px, 1fr)`) / `dock` (auto-width side panel), with a full-width `status` bar (30px) beneath. The main workspace itself stacks a fixed-height header (64px) over scrollable content. Density is high: sidebar items are 34–36px tall, buttons 30px (25px for `.sm`), section rows ~29px. Spacing is tight and consistent — 6–9px internal padding is the norm for interactive rows, 13–22px for panel/modal padding. The admin UI reuses the same grid and spacing rhythm so the two surfaces read as one product.

## Elevation & Depth

Flat by default. Structural depth between surfaces comes from tonal layering — `bg` → `surface` → `surface-raised` → `surface-hover` → `surface-active` — not from shadow. `box-shadow` (`--shadow-lg`, plus a few bespoke soft shadows) is reserved for elements that visibly float above the normal layout: modals, toasts, the mention menu, the scroll-to-latest pill, and the workspace-switching indicator. At-rest content — sidebar items, cards, chat bubbles, buttons — never carries a shadow.

### Shadow Vocabulary
- **Overlay** (`--shadow-lg`: `0 24px 70px rgba(0,0,0,.42), 0 2px 10px rgba(0,0,0,.22)`): modals, mention menu, toasts, tooltips — the standard "floating above everything" shadow.
- **Ambient lift** (e.g. `0 4px 8px rgba(0,0,0,.28)`, `0 7px 22px rgba(0,0,0,.3)`): smaller transient elements (workspace-switching pill, scroll-to-latest pill) that float but sit lower in the stack than a modal.

### Named Rules
**The Flat-By-Default Rule.** Surfaces at rest are flat and distinguished only by tone. Shadow is added exclusively as a response to an element leaving the document's normal stacking context (fixed/absolute overlays), never as decoration on a resting card or button.

## Shapes

Radius is small and consistent: `--radius-sm` (7px) for buttons, inputs, and small controls; `--radius` (10px) for cards and mid-size containers; `--radius-lg` (14px) for modals and larger panels. Fully round (`999px`/`50%`) is reserved for pills (chips, status badges, the running-session dot) and circular icon targets (avatars, the sidebar help button). Borders are 1px, almost always `--border` or `--border-soft`, and are the primary way containers are delimited — not shadow, not heavier fills.

## Components

Buttons, chips, and inputs are quiet and confident: restrained color (the accent appears only on the primary button and active/focus states), flat at rest, with short 120–140ms transitions on hover rather than motion for its own sake.

### Buttons
- **Shape:** 7px radius (`--radius-sm`); `.lg` variant steps up to a taller 42px min-height, `.sm` down to 25px.
- **Default:** `surface-raised` background, `text-soft` color, 1px `border`; hover shifts to `surface-hover`/`text`/a slightly lighter border.
- **Primary:** solid Burnt Terracotta background, near-black (`#201713`) text; hover shifts to Burnt Terracotta Bright.
- **Danger:** transparent-tinted red wash (`rgba(216,111,114,.09)`) with red text and a soft red border — never a solid red fill.
- **Active/press:** `translateY(1px)` on `:active`, no shadow change.

### Chips
- **Style:** pill radius (999px), transparent background, `muted` text, 1px `border`.
- **State:** `.on` (selected/active) fills with `surface-active` and switches text to `--text` with a stronger border; hover on unselected chips lightens text and adds `surface-hover`.

### Cards / Containers
- **Corner Style:** 10px radius (`--radius`) for standard cards/panels; 14–15px for modals and larger feature cards (e.g. chat-empty-mark, gate-card).
- **Background:** `surface` for resting cards, `surface-raised` for anything meant to sit "on top" (composer, modal, popovers).
- **Shadow Strategy:** none at rest; see Elevation & Depth for floating elements.
- **Border:** 1px, `border` or `border-soft`.
- **Internal Padding:** 13px for compact cards (prompt chips, tool/subagent blocks), 21–22px for modals, up to 36–42px for centered feature cards (gate-card).

### Inputs / Fields
- **Style:** filled (`#292925`/`surface-active`-family background), 1px transparent border by default, 7–8px radius.
- **Focus:** border shifts to a lighter neutral (`#4b4b44`) plus a subtly lighter background; interactive elements broadly use a 2px accent-tinted outline (`rgba(238,146,120,.75)`) via `:focus-visible`.
- **Placeholder:** always `--faint`, never `--muted` or darker.

### Navigation (Sidebar)
- **Style:** 8–9px radius nav rows, `text-soft`/`muted` icon color at rest, shifting to `text`/`text-soft` on hover, `surface-active` fill plus `text` color when active. Section titles are 12px/600-weight `--muted` labels with an optional inline add/action control that only shows real affordance on hover.
- **Mobile/compact treatment:** a `data-compact-sidebar` mode reduces item height and hides secondary metadata rather than reflowing to a different pattern.

### Status Signals (signature component)
A recurring 6px `.dot` communicates run state across both surfaces: neutral `--faint` at rest, `--green` with a soft pulsing glow (`box-shadow` + `pulse` keyframe) when a session is actively running. The same status-color vocabulary (green/yellow/red/blue, each with a matching low-opacity "soft" wash in the admin UI) extends to badges and approval/tool-call blocks, so state is always legible by color alone before the label is read.

## Do's and Don'ts

### Do:
- **Do** reserve Burnt Terracotta for primary actions and active/running state; everything else stays neutral or a fixed status color.
- **Do** express depth with tonal layering (`bg`/`surface`/`surface-raised`/`surface-hover`/`surface-active`) and reach for `--shadow-lg` only when an element leaves normal document flow (modal, toast, popover, floating pill).
- **Do** keep the type scale's odd weights and half-pixel sizes (540/560/580/620; 10.5px/11.5px/12.5px) rather than rounding to nearest 100/whole pixel.
- **Do** pair micro-label text (timestamps, hints, badges) with `--muted` or `--faint`, never full `--text` color.
- **Do** keep the desktop and admin surfaces on the same token set — the admin UI is explicitly built to match `vak-desktop`, not to diverge stylistically.

### Don't:
- **Don't** add box-shadow to resting cards, buttons, or sidebar items — shadow means "this is floating," not "this is important."
- **Don't** introduce a second accent color or use a status color (green/yellow/red/blue) decoratively outside its state meaning.
- **Don't** reach for gradients or glassmorphic blur outside the few already-established floating overlays (toast, scroll-latest pill, mention menu) that use `backdrop-filter: blur(...)` deliberately.
- **Don't** use pure black/white; every "black" is the warm `--bg` (`#171714`) family and every "white" is warm `--text` (`#eeeae2`), never `#000`/`#fff`.
