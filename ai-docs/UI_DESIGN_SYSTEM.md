# UI_DESIGN_SYSTEM — "Strongroom"

Visual and interaction system for the Passman UI (`src/`). Dark only. Written before the components; the code in `src/styles/` and `src/components/` implements exactly this. Backend boundary: `IPC_CONTRACT.md`.

## 1. Direction

Passman is opened for about ten seconds at a time, usually to copy one value, and the person must feel sure nothing leaks. The look is a **strongroom**: deep marine ground, frosted glass panes, a single brass "key" accent, tactile raised surfaces. References for quality (not for look-alikes): 1Password, Proton Pass, Apple Keychain.

Calibration notes (what we deliberately avoided): aurora/violet gradient glows, one identical rounded card per field, all-caps eyebrow labels, monospace for ordinary labels, `A · B · C` meta strings, arrows appended to buttons, emoji as icons.

The one memorable element is the **unlock moment** (§6). The recurring motif is a faint **triangular lattice** in the ground (a nod to the lattice-based ML-KEM), which also gives the glass something real to blur.

## 2. Tokens

Defined once in `src/styles/tokens.css`; Tailwind utilities are generated from them (`bg-pane`, `text-mist`, `rounded-lg` ...).

### Colour (contrast measured against the real surfaces, WCAG 2.x)

| Token | Hex | Role | Contrast on pane / raised / hover |
|-------|-----|------|-----------------------------------|
| `marine` | `#08141A` | window ground | – |
| `pane` | `#0E1D25` | pane surfaces | – |
| `raised` | `#152A34` | raised rows, inputs | – |
| `hover` | `#1B3541` | hover / selected | – |
| `frost` | `#E6F0F2` | primary text | 14.8 / 12.8 / 11.1 |
| `mist` | `#9BB2B9` | secondary text | 7.8 / 6.7 / 5.8 |
| `fog` | `#8AA1A8` | tertiary text, hints | 6.3 / 5.5 / 4.7 |
| `brass` | `#E2B259` | the key: primary action, focus ring, selection edge | 8.8 / 7.6 / 6.6 |
| `brass-hi` / `brass-lo` | `#EDC378` / `#CC9A45` | hover / pressed | – |
| `on-brass` | `#231905` | text on brass fills | 8.9 on brass |
| `verdigris` | `#52D3B8` | secure/healthy state, fresh TOTP window | 9.3 / 8.1 / 7.0 |
| `ember` | `#F0737A` | danger text, low TOTP time | 6.1 / 5.3 / 4.6 |
| `danger` | `#C23B44` | destructive button fill (white text 5.3) | – |

Glass: fill `rgba(15, 32, 41, 0.58)`, hairline `rgba(190, 230, 240, 0.10)`, `backdrop-filter: blur(22px) saturate(1.35)`. Text over glass is always `frost`/`mist` and was measured on the lightest lattice point.

Colour is never the only signal: states also change shape, icon or text (e.g. TOTP low time also shortens the arc and pulses the digits once).

### Type

| Role | Family | Size / line | Weight | Used for |
|------|--------|-------------|--------|----------|
| Display | Fraunces | 34 / 38 | 580 | lock screen wordmark, empty states |
| Title | Fraunces | 26 / 30 | 560 | item title in the detail pane |
| Heading | Instrument Sans | 17 / 22 | 620 | group headings, sheet titles |
| Body | Instrument Sans | 15 / 22 (14.5 on desktop) | 450 | rows, values |
| Caption | Instrument Sans | 12.5 / 16 | 500 | meta ("Edited 2 days ago") |
| Secret | IBM Plex Mono | 15 / 22 | 500 | passwords, note bodies |
| Code | IBM Plex Mono | 30 / 34, tracking 0.08em | 500 | TOTP digits |

Sentence case everywhere. Line length stays under 70 characters (detail column max `34rem` of text). Fonts are bundled locally (`@fontsource*`), never fetched (CSP `font-src 'self'`).

### Space, radius, elevation

* Space: 4 px base, steps 4 · 8 · 12 · 16 · 20 · 24 · 32 · 48. Touch targets on `(pointer: coarse)` are at least 48 × 48 px; desktop controls are 32–36 px.
* Radius is tiered by hierarchy, not uniform: **8** (inputs, buttons, chips) · **14** (rows, grouped panels) · **22** (sheets, lock card, glass shells) · full (avatars, FAB, rings).
* Elevation (`--elev-1..4`) combines a top inner highlight, a 1 px dark bottom edge and a teal-tinted ambient shadow, so surfaces read as machined metal, not floating cards. Pressed controls sink 1 px and lose their highlight. The primary button has a subtle vertical tone shift (material, not decoration).

### Motion

Tokens: `--dur-fast 120ms`, `--dur 200ms`, `--dur-slow 340ms`, `--ease-out cubic-bezier(.2,.8,.2,1)`, spring for sheets `{ stiffness: 420, damping: 38 }`. `MotionConfig reducedMotion="user"` plus a CSS `prefers-reduced-motion` override: transitions collapse to opacity, rings update without easing.

## 3. Platform adaptation

One codebase, three layouts chosen by viewport width (`useLayout`), plus input-aware sizing (`pointer: coarse`):

| Layout | Width | Structure |
|--------|-------|-----------|
| Wide (Linux desktop) | ≥ 1100 px | Sidebar 236 · List 344 · Detail (flex). Editor slides in from the right as a glass drawer. Keyboard: `/` or `Ctrl K` focuses search, arrows move in the list, `Esc` closes drawers. |
| Medium | 720–1099 px | Icon rail 64 · List 320 · Detail. |
| Compact (Android) | < 720 px | One column. Bottom navigation (Vault · Codes · Trash · Settings), a thumb-reach FAB above it, detail as a full page that slides in, add/edit/re-prompt as drag-to-dismiss **bottom sheets** with a handle. Respects `env(safe-area-inset-*)`. Android back closes sheet → detail → list (History API). |

Lock screens: master password (all platforms); Quick PIN pad and biometric button are built but **shown only when `VaultStatus.capabilities` says they work** (false until Phase 3), so the UI never offers an unlock path the core cannot honour. In `npm run dev` in a browser the preview backend enables them so the designs can be reviewed.

Native follow-ups that need Rust/Android work and are not part of this UI pass: `FLAG_SECURE` on sensitive screens, BiometricPrompt, window-blur lock (Phase 3).

## 4. Screens and components

* **Lock / create vault** — glass card on the lattice. Create: master password + confirm, live strength meter (length + character classes, honest "no recovery" notice), then the unlock moment.
* **Vault** — sidebar (All items, Logins, Codes, Notes, Media, Favorites, Trash, Settings; footer shows the real sync state "Local only"), list (search, grouped by recency, selection edge in brass that glides between rows with a shared layout animation), detail.
* **Detail** — Fraunces title, avatar tile (monogram on a hue derived from the title), one grouped panel of fields separated by hairlines (not a card per field). Each secret is a frosted pill; **Reveal** is password-gated (re-prompt sheet), auto-hides after 20 s with a shrinking ring; **Copy** on a secret works inside the reveal window. TOTP: live ring + grouped digits + seconds, the secret never reaches the UI. Below: Attachments (list; adding arrives with Phase 5, so the add button is disabled with a plain explanation) and a collapsible **Version history** (v1, v2, … with exact time and changed field names; viewing a snapshot's secrets and **Restore this version** are password-gated).
* **Editor** — type picker for new items; fields per type from the contract's schema; password generator (length slider, character classes, uses `crypto.getRandomValues` with rejection sampling); custom fields; tags; favourite. Saving a changed item goes through `item_update` with the edit token.
* **Re-prompt** — one promise-based component for every gated action: states the reason ("Reveal the password for GitHub"), shows progress while Argon2id runs (it is slow on purpose), shakes once and keeps the sheet open on a wrong password.
* **Codes** — all TOTP items as a grid with live rings (tab on mobile, filter on desktop).
* **Trash** — rows show only the title and "Deleted <time>". The detail area states that the item must be restored to open it. **Delete forever** is a two-step flow: warning, then master password plus typing the item name or `CONFIRM`. Nothing auto-expires and the copy says so.
* **Settings** — clipboard clear (15 / 30 / 60 s, default 30), change master password, lock now, about (suite from `crypto_suite_info`). Lock timers, quick unlock and sync controls are added when their backends exist, not before.
* **Feedback** — toasts with Undo for trash; a copy toast with a ring that counts down to clipboard clearing; empty states that say what to do next.

## 5. Security-driven UX rules

1. The webview is untrusted for secrets: it holds a secret only inside an open reveal/edit window, and clears it afterwards (state reset, not just hidden).
2. Re-prompt for edit, reveal, history secrets, restore-from-history and permanent delete, exactly as the spec says. Create, trash and restore-from-trash are not gated.
3. Trash is restricted in Rust; the UI additionally never offers actions that would fail.
4. Clipboard: copying a secret schedules a clear after the configured time; the toast shows the countdown. Non-secret values (username, url) are not auto-cleared.
5. No telemetry, no external requests, no inline scripts. Fonts and icons are bundled.
6. Honest status: the sidebar says "Local only" until a sync target exists; unavailable features are absent or explained, never faked.

## 6. The unlock moment (the single orchestrated animation)

On success: the lock card's inner ring rotates a quarter turn and a brass segment completes the circle (240 ms), the card scales to 0.97 and fades (200 ms), then the three panes rise 10 px into place with a 40 ms stagger. Failures shake the card horizontally (±6 px, 2 cycles). Nothing else animates on its own except live rings (TOTP, reveal timer, clipboard countdown).

## 7. Accessibility

Visible focus ring (2 px brass with 2 px offset) on every interactive element; dialogs are `role="dialog" aria-modal`, trap focus, close on `Esc`, restore focus; lists are `role="listbox"` with arrow-key support; rings expose text alternatives (`role="timer"`, "17 seconds left"); toasts use `aria-live="polite"`; hit areas ≥ 48 px on touch; colour never the sole signal; reduced motion respected.

## 8. Code layout

```
src/
  main.tsx, App.tsx          providers, phase switch (boot → create/lock → app)
  styles/                    tokens.css (design tokens), base.css (ground, lattice, utilities)
  backend/                   types.ts (contract), tauri.ts (invoke), mock.ts (preview), index.ts
  state/                     store.ts (tiny external store), actions.ts
  lib/                       format, password (generate + strength), totp ring math, clipboard, layout hook
  components/ui/             Button, IconButton, Field, Switch, Segmented, Sheet, Modal, Toast, Avatar, Ring, Glass
  components/lock/           LockScreen, CreateVault, PinPad
  components/shell/          Shell, Sidebar, RailNav, BottomNav, Fab
  components/items/          ItemList, ItemDetail, FieldRow, SecretValue, TotpRing, VersionHistory, Attachments, ItemEditor, PasswordGenerator
  components/trash/          TrashView, PurgeFlow
  components/reprompt/       RepromptHost (promise API)
  components/settings/       SettingsView
```

`src/backend/index.ts` picks the Tauri backend when `isTauri()` is true. The **preview backend** (in-memory, fake password check, real TOTP maths) is loaded only in development outside Tauri, behind `import.meta.env.DEV`, so it is not part of production bundles. It exists to iterate on the design in a normal browser and to drive screenshots; it is **not secure** and says so.

Dev-only deep links for screenshots (preview backend only): `?vault=unlocked|locked`, `&open=<n>` (nth item), `&sheet=edit|new|reprompt`, `&filter=trash|codes|settings`, `&history` (opens the version timeline). Motion animations do not finish under a short headless virtual-time budget; use a budget of about 20 s when a sheet or timeline must be fully open in a screenshot.

## 9. CSP

`style-src 'self'` blocks `<style>` blocks and HTML `style=""` attributes, not styles set through the CSSOM. React's `style` prop and Motion animate via CSSOM. Verified so far in the real WebKitGTK window: the first-run screen (bundled fonts, lattice, glass, the SVG ring) renders correctly under the strict CSP. Not yet verified there: running Motion transitions (sheets, layout animations, the unlock moment); that is part of the manual smoke test in `CHANGELOG_AND_STATE.md`. Never add inline `<script>`/`<style>` tags or `dangerouslySetInnerHTML`.
