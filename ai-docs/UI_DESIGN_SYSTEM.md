# UI_DESIGN_SYSTEM — "Strongroom"

Visual and interaction system for the Hexis UI (`src/`). Dark and light (§2 and §10; the person chooses, or the device decides). Written before the components; the code in `src/styles/` and `src/components/` implements exactly this. Backend boundary: `IPC_CONTRACT.md`.

## 1. Direction

Hexis is opened for about ten seconds at a time, usually to copy one value, and the person must feel sure nothing leaks. The look is a **strongroom**: deep marine ground, flat panes with hairline edges, a single brass "key" accent. (The first version used frosted glass and a lattice; the Q5 pass in §9 removed both.) References for quality (not for look-alikes): 1Password, Proton Pass, Apple Keychain.

Calibration notes (what we deliberately avoided): aurora/violet gradient glows, one identical rounded card per field, all-caps eyebrow labels, monospace for ordinary labels, `A · B · C` meta strings, arrows appended to buttons, emoji as icons.

The one memorable element is the **unlock moment** (§6). The signature detail is the **brass key beside keycaps**: the key button is the one brass surface on a screen, and every keyboard shortcut is written as a small keycap.

## 2. Tokens

Defined once in `src/styles/tokens.css`; Tailwind utilities are generated from them (`bg-pane`, `text-mist`, `rounded-lg` ...).

### Colour (contrast measured against the real surfaces, WCAG 2.x)

There are two complete palettes, dark and light (D37); this table is the dark one. The light palette is below it.

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

**Light palette** (`:root[data-theme="light"]`, and `prefers-color-scheme: light` when no theme is chosen):

| Token | Hex | Role | Contrast on pane / raised / hover |
|-------|-----|------|-----------------------------------|
| `marine` | `#E7EEF0` | window ground | – |
| `pane` | `#F4F8F9` | pane surfaces | – |
| `raised` | `#FFFFFF` | raised rows, inputs | – |
| `hover` | `#DCE7EA` | hover / selected | – |
| `frost` | `#0F2630` | primary text | 14.7 / 15.7 / 12.4 |
| `mist` | `#41575F` | secondary text | 7.1 / 7.6 / 6.1 |
| `fog` | `#52676E` | tertiary text, hints | 5.6 / 6.0 / 4.7 |
| `brass` | `#D9A23A` | the key button fill | – (label `on-brass` `#231905`: 7.6) |
| `brass-ink` | `#7D550B` | the accent as text, focus ring, selection edge, dots | 6.2 / 6.6 / 5.2 |
| `verdigris` | `#0A7360` | secure/healthy state | 5.4 / 5.8 / 4.6 |
| `ember` | `#B4303A` | danger text | 5.7 / 6.1 / 4.9 |
| `danger` | `#C23B44` | destructive button fill (white text 5.2) | – |

`brass-ink` is new: the accent used as text, a line or a dot. In the dark palette it equals `brass`; in the light one it is the darker `#7D550B`, because the gold fill is too pale to read as text on white. `brass` (the fill of the key button) is gold in both. Hairlines are the ink colour at 12% and 22% (`rgb(16 48 60 / …)`) instead of the pale tint, and shadows are small and neutral (`--shadow-e1` … `e4`, plain variables with hand-written `shadow-e1`/`shadow-e2` utilities, because Tailwind would inline a theme shadow and make it unthemeable). The avatar hues, the ring track, selection, scrollbar, the key's disabled look and the pale "frosted" secret pill are variables too. The logo's gradient (an existing brand asset) is the same in both. `<meta name="color-scheme" content="light dark">` and `color-scheme` on the root keep form controls and scrollbars in step.

Surfaces (`surface` utility, was `glass`): a solid fill from the marine ramp and a hairline `rgba(190, 230, 240, 0.10)`. No `backdrop-filter`, no lattice, no coloured glow. Text on a surface is always `frost`/`mist`.

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
* Elevation is a surface step plus a hairline; only a floating sheet or menu keeps a small neutral shadow. Pressed controls sink 1 px. (The first version's top highlights, brass glow and teal ambient shadows were removed in Q5.)

### Motion

Tokens (revised in Q5): `--dur-fast 80ms`, `--dur 120ms`, `--dur-slow 180ms`, one easing `cubic-bezier(.2,.8,.2,1)` (`src/lib/motion.ts` exports `EASE`, `DUR` and `tween` for Motion). No springs. `MotionConfig reducedMotion="user"` plus a CSS `prefers-reduced-motion` override: transitions collapse to opacity, rings update without easing.

## 3. Platform adaptation

One codebase, three layouts chosen by viewport width (`useLayout`), plus input-aware sizing (`pointer: coarse`):

| Layout | Width | Structure |
|--------|-------|-----------|
| Wide (Linux desktop) | ≥ 1100 px | Sidebar 236 · List 344 · Detail (flex). Editor slides in from the right as a glass drawer. Keyboard: `/` or `Ctrl K` focuses search, arrows move in the list, `Esc` closes drawers. |
| Medium | 720–1099 px | Icon rail 64 · List 320 · Detail. Type filters become chips above the list. |
| Compact (Android) | < 720 px | One column. Bottom navigation (Vault · Codes · Generator · Settings; Trash, Archive, Import, Export and Backup are reached from the type chips and the Settings page), a thumb-reach FAB above it that creates the type of the current filter, detail as a full page that slides in, add/edit/re-prompt as drag-to-dismiss **bottom sheets** with a handle. Respects `env(safe-area-inset-*)`. Android back closes sheet → detail → list (History API). |

Lock screens: an **account picker** when several accounts exist (one tile per account, most recently used first), then that account's master password (all platforms); Quick PIN pad and biometric button are built but **shown only when `VaultStatus.capabilities` says they work** (false until Phase 3), so the UI never offers an unlock path the core cannot honour. In `npm run dev` in a browser the preview backend enables them so the designs can be reviewed.

Native follow-ups that need Rust/Android work and are not part of this UI pass: `FLAG_SECURE` on sensitive screens, BiometricPrompt, window-blur lock (Phase 3).

## 4. Screens and components

* **Lock** — glass card on the lattice. First run offers *Create a new vault*, *Open a vault file* and *Restore from a backup*. With accounts it shows the picker, then the chosen account's avatar and password field. Create: unique account name (live check), master password + confirm, live strength meter (honest "no recovery" notice), then **"Back up this vault?"** (set up now or skip; skipping is fine and the Backup page says so), then the unlock moment.
* **Accounts** — one local vault per account, each with its own master password. Names are unique per device (case, spacing and width-insensitive). The sidebar chip (avatar on rail/compact) opens the account sheet: switch (locks first), add, lock now. Settings → Accounts renames and deletes (delete needs that account's password plus its name or `CONFIRM`).
* **Vault** — sidebar groups: items by type (All, Logins, Cards, Identities, SSH keys, Notes, Media) · Codes, Favorites, Archive, Trash · Generator, Import, Export, Backup · Settings. Counts per entry, a real backup-status chip ("Local only", "Backed up 3 min ago", "Backup needs attention"), list (search, grouped by recency, selection edge in brass that glides between rows), detail.
* **Item menu** (Bitwarden-style) — right-click, long-press (480 ms) or the "more" button: View, Edit, Clone, Archive/Unarchive, Move to trash, Launch (only when the item has a web address; opens the system browser, never the webview), and quick copies per type (Login: username, password; Card: number, security code; Identity: email, phone; SSH key: public key, private key). Secrets in quick copies and Clone go through the re-prompt. Desktop: popover with arrow-key navigation. Compact: bottom sheet.
* **Item types** — Login, Card, Identity, SSH key, Secure note, Media, each with its own field set (`paragraph` fields render as mono text; secret fields are masked until revealed). Archived items leave every list and Codes, and live only under Archive (banner with Unarchive on the detail).
* **Detail** — Fraunces title, avatar tile (monogram on a hue derived from the title), one grouped panel of fields separated by hairlines. Each secret is a frosted pill; **Reveal** is password-gated, auto-hides after 20 s with a shrinking ring; **Copy** works inside the reveal window. Web addresses offer "Open in your browser". TOTP: live ring + grouped digits + seconds; the setup key never reaches the UI. Below: Attachments (adding arrives with Phase 5) and a collapsible **Version history**.
* **Editor** — type picker (7 types), fields per type, password generator for login passwords and SSH passphrases only, custom fields, tags, favourite. The one-time-code field accepts anything: an `otpauth://` link, a bare key (any case, spaces, dashes, `secret=` prefix), a web link carrying `?secret=`, a Google Authenticator `otpauth-migration://` export (split into one item per account on a Code-only item), or a QR image (file, paste, or drop). On Android there is also **Scan QR code**, in-app, asking for the camera permission. The field says what it recognised ("GitHub, 6 digits, every 30 s") and the Rust core still validates on save.
* **Codes** — every item with a one-time code: logins that have one *and* Code-only items, with an "In a login" / "Code only" badge, live rings, search when there are many, "Add code". Tapping a row opens its item.
* **Generator / Import / Export** — full pages. Generator: the same generator as the editor, standalone; both start from the last settings the person chose (length 8 to 64 on the slider, character types), kept in the vault and synced (D31, `IPC_CONTRACT.md` §12). Import: Bitwarden JSON or a Chrome/Firefox/1Password CSV, with a preview, a list of skipped rows and warnings; items are sealed one by one and the parsed plaintext is dropped afterwards. Export: an **encrypted** Hexis vault file saved through a native dialog behind the re-prompt. There is no plaintext export, by design.
* **Backup** — multiple S3 or S3-compatible targets (Amazon S3, Cloudflare R2, Backblaze B2, other). Each row shows a health dot and state, queue, last success and error, with Back up now, Edit, Make master, Copy secret key (re-prompt) and Remove. Two checkboxes in the form: **Also keep these credentials in the master backup** and **Use this as the master backup**. Credentials are stored sealed in the local vault database, so they survive every lock and are never asked for again.
* **Re-prompt** — one promise-based component for every gated action: states the reason, shows progress while Argon2id runs, shakes once and keeps the sheet open on a wrong password.
* **Trash** — rows show only the title and "Deleted <time>". The detail states that the item must be restored to open it. **Delete forever** is two steps: warning, then master password plus typing the item name or `CONFIRM`. Nothing auto-expires and the copy says so.
* **Settings** — Accounts, clipboard clear (15 / 30 / 60 s, default 30), change master password, "Backup and files" links (Backup, Import, Export), lock now, about (suite from `crypto_suite_info`). Lock timers and quick unlock are added when their backends exist.
* **Feedback** — toasts with Undo for trash and archive; a copy toast with a ring that counts down to clipboard clearing; per-filter empty states that say what to do next.
* **App lock gate (Stage 1)** — optional, set up in Settings → App lock. Full-screen card with one decoy: an arithmetic exercise (`(48 + 27) − 19 = ?`, Fraunces, tabular figures) above an answer box and the credential entry of the chosen mode: **PIN** keypad, **password** field (`inputMode="text"`), or the **numbered 3×3 grid** (a tap types the digit; dragging across two or more numbers draws a brass line and sends the sequence on release). Digits stay readable for the first three characters (they might be a maths answer) and are masked from the fourth. "Correct!" / "Incorrect, try again" are quiet one-liners; a wrong passcode of four or more characters shows a red banner with the exact attempts left (16 in all). After the 3rd, 6th, 9th, 12th and 15th wrong attempt the keypad refuses passcodes for a while and a panel counts the time down (1.5 min, 5 min, 45 min, 8 h, 3 days; "Nothing you type as a passcode is checked until then"). Before the last attempt a red "Last attempt" panel says a wrong passcode erases everything, and sending it asks "Send your last attempt?" with the number of digits typed. Biometric button only when the platform offers it. The reset is deliberately discreet: a small "?" mark and a long-press on the equation open a **three-step** confirmation (three different warnings; the last one shows a long sum and an answer field, and only a right answer enables the single "Erase everything" to work; a wrong answer erases nothing, says so and shows a new sum). The optional panic mark is a 10 px dot at the lower right that needs a 1.2 s hold or a double tap. A panic credential, a panic mark or the 16th failure ends on the first-run screen with no banner, toast or animation. The gate card never shows the app name or the Hexis mark.
* **App icon and name** — Settings → App icon and name: the 11 presets as a radio grid of tiles (the original first), a lower-case switch, and a custom-name field. The chosen name and icon replace "Hexis" and the keyhole inside the app (sidebar, lock screens, window title). `assets/disguise/presets.json` is the source of the ids, names and tile colours; the icons are generated by `scripts/gen_disguise_icons.py`.
* **Files** — sidebar entry and Settings link: list of encrypted files with Preview, Save copy, Rename and Delete (re-prompt), add up to 50 MB each. Item attachments use the same list inside the item detail. Preview sheet: images in an `<img>` from a blob URL, text in a mono block (first 2 MB), PDFs drawn page by page on canvases with pdf.js (loaded only when a PDF is opened). Bytes exist in the webview only while a preview is open.
* **Import and export** — Import is a guided page: choose a file (the core opens the dialog and names the format from the content), confirm or correct the format, give a KeePass password and optional key file, match columns for an unknown CSV, pick how duplicates are handled, check the file (counts, warnings, names), import, then a reminder to delete the unencrypted source. Export offers the encrypted vault file and, behind a danger re-prompt and a plain warning, a readable Bitwarden JSON or CSV.
* **QR scanning (mobile)** — the barcode-scanner plugin renders the camera *behind* a transparent webview (`windowed: true`); `ScanOverlay` and the `html[data-scanning]` rules in `base.css` hide everything else and draw the frame, a hint and Cancel. Images are decoded with jsQR on every platform. **Not yet verified on a device** (no Android SDK on the build machine).

## 5. Security-driven UX rules

1. The webview is untrusted for secrets: it holds a secret only inside an open reveal/edit window, and clears it afterwards (state reset, not just hidden).
2. Re-prompt for edit, reveal, history secrets, restore-from-history, permanent delete, Clone, copying a secret from the item menu, copying a backup secret key, exporting a vault file and deleting an account. Create, archive/unarchive, trash and restore-from-trash are not gated.
3. Trash is restricted in Rust; the UI additionally never offers actions that would fail.
4. Clipboard: copying a secret schedules a clear after the configured time; the toast shows the countdown. Non-secret values (username, url) are not auto-cleared.
5. No telemetry and no inline scripts. Fonts and icons are bundled. The webview makes no network requests at all; the only traffic leaving the machine is the Rust core talking to the S3 endpoints the user configured and, unless the person switches it off, the three-site time check (`ARCHITECTURE.md` D27). Web addresses are opened by the core in the system browser, never inside the webview.
6. Honest status: the sidebar says "Local only" until a backup target exists, then reports real health and the time of the last successful upload; unavailable features are absent or explained, never faked.

## 6. The unlock moment (the single orchestrated animation)

On success: the Hexis mark closes its two layers (the glass cyan layer of the crossbar slides up into the silver one, 240 ms) and grows 5 %, the card scales to 0.97 and fades (200 ms), then the three panes rise 10 px into place with a 40 ms stagger. Failures shake the card horizontally (±6 px, 2 cycles). Nothing else animates on its own except live rings (TOTP, reveal timer, clipboard countdown).

## 7. Accessibility

Visible focus ring (2 px brass with 2 px offset) on every interactive element; dialogs are `role="dialog" aria-modal`, trap focus, close on `Esc`, restore focus; lists are `role="listbox"` with arrow-key support; rings expose text alternatives (`role="timer"`, "17 seconds left"); toasts use `aria-live="polite"`; hit areas ≥ 48 px on touch; colour never the sole signal; reduced motion respected.

## 8. Code layout

```
src/
  main.tsx, App.tsx          providers, phase switch (boot → welcome/picker/locked → ready)
  styles/                    tokens.css (design tokens), base.css (ground, lattice, utilities, scan-mode rules)
  backend/                   types.ts (contract), tauri.ts (invoke), mock.ts (DEV preview), http.ts (DEV e2e), index.ts
  state/                     store.ts, app.ts (phases, accounts, items, gate), backup.ts, brand.ts, activity.ts, ui.ts, dev.ts (DEV deep links)
  lib/                       format, password, totp ring math, otp.ts (every setup-key shape), scan.ts (QR), clipboard, layout hook, import parsers
  components/ui/             Button, Inputs, Sheet, Toasts, Avatar, Ring
  components/lock/           LockScreen (chooser, picker, unlock), CreateFlow, JoinFlows (open file, restore), NameField, PinPad
  components/shell/          Shell, Navigation (Sidebar, Rail, BottomNav, Fab), nav.ts, AccountMenu, FilterChips, PageHeader
  components/items/          ListPane, ItemRow, ItemMenu, ItemDetail, ItemEditor, TotpInput, CodesView, actions.ts, typeMeta.ts, FieldRows, VersionHistory, PasswordGenerator
  components/gate/           GateScreen, GridPad, ResetFlow
  components/files/          FilesView, FileList, FilePreview
  components/tools/          GeneratorView, ImportView, ExportView
  components/backup/         BackupView, TargetForm
  components/settings/       SettingsView, Accounts, AppLock, Disguise
  components/trash/          TrashPane (list, detail, purge flow)
  components/reprompt/       Reprompt (promise API)
  components/scan/           ScanOverlay
```

`src/backend/index.ts` picks the Tauri backend when `isTauri()` is true. The **preview backend** (in-memory, fake password check, real TOTP maths) is loaded only in development outside Tauri, behind `import.meta.env.DEV`, so it is not part of production bundles. It exists to iterate on the design in a normal browser and to drive screenshots; it is **not secure** and says so.

Dev-only deep links for screenshots (preview backend only): `?vault=unlocked|locked|picker`, `&account=<name>`, `&filter=<any filter>`, `&open=<n>` (nth item), `&sheet=edit|new|reprompt`, `&type=<item type>` (with `sheet=new`), `?vault=gate&gate=pin|password|grid[&panic=1]` (the gate; preview credentials 24680, abcd, 12589), `&disguise=<preset id>[&lower=1]`. Motion animations do not finish under a short headless virtual-time budget; use a budget of about 20 s when a sheet or timeline must be fully open in a screenshot. `?e2e=<port>` (DEV only) points the UI at the loopback HTTP shim around the real Rust core, so the whole app can be driven in headless Chromium against real cryptography and real files; it is compiled out of production builds.

## 9. CSP

`style-src 'self'` blocks `<style>` blocks and HTML `style=""` attributes, not styles set through the CSSOM. React's `style` prop and Motion animate via CSSOM. Verified so far in the real WebKitGTK window: the first-run screen (bundled fonts, lattice, glass, the SVG ring) renders correctly under the strict CSP. Not yet verified there: running Motion transitions (sheets, layout animations, the unlock moment); that is part of the manual smoke test in `CHANGELOG_AND_STATE.md`. Never add inline `<script>`/`<style>` tags or `dangerouslySetInnerHTML`.

## 9. The Q5 pass (2026-11-24): shortcuts, layered settings, zone page, demo vault, less "AI" look

**Direction (written with the `unslop` method).** Product: a private, local-first, post-quantum password vault. Character: quiet, exact, a little austere. World: a strongroom, a brass key, a ledger. Moments: the unlock, and the demo vault's list. Signature: the brass key beside keycaps. **Removed:** glass blur, the lattice, brass glow and gradients, the pulse, bevel highlights, brass-tinted fills, exclamation marks and decorative icons. **Not changed:** the logo's gradient (an existing brand asset).

**Motion.** Every transition is 80, 120 or 180 ms on one easing. The pattern lock (`GridPad`) does not animate at all: no fade, no dimming while a code is checked; the line follows the finger through one SVG attribute written in `requestAnimationFrame`, and React re-renders only when a number is entered. A compact detail page slides in 160 ms. Reduced motion still collapses everything to opacity.

**Shortcuts** (`src/lib/shortcuts.ts` is the one table; the overlay, Help and the handler in `components/shell/shortcuts.ts` all read it). 33 shortcuts in four groups: General, Go to, Items, Selected item. Writing: single keys (`n`, `/`, `?`), modifier combos (`Mod+K`, `Mod+Shift+L`; `Mod` is Ctrl, or Cmd on Apple platforms) and two-key chords (`g` then `a`, 1.5 s window). Single keys and chords are ignored while typing in a field or while a dialog is open; modifier combos, `F1` and `Esc` work anywhere. `?` or `F1` opens the overlay; Settings > Help and shortcuts shows the same table and the common questions.

**Settings are layered.** Eight pages: Security, App lock, Accounts, Date and time, App icon and name, Backup and files, Help and shortcuts, About. A phone (compact layout) shows a list of pages with a one-line state each and drills into one page with a back control (the system back gesture too); a tablet and a desktop show the list as a column on the left and the page on the right. The page is kept when the person returns from Import, Export, Backup or Files (`SETTINGS_CHILDREN`).

**First start.**
1. *Choose your time zone* is always the first page: a searchable list (place name or `utc+06`), the device's zone suggested, no skip. Nothing is saved before Continue. A lock-screen gate, when one exists, comes before it.
2. The app opens the demo vault "Hexis123" (a "Opening your vault" splash while it is made).
3. A sheet, "You are in a demo vault", once: *Create my own vault*, *Open a vault file*, *Restore from a backup*, *Explore the demo*. Until it is answered the demo is labelled; after that it looks like any other vault. An existing install with vaults gets the zone page once and no sheet.
4. After an erase or a forgotten-password reset the zone and the "setup done" flag survive (`device.json`), so the app opens straight into a fresh demo vault with no page and no label.

**Lock screen.** A wide screen shows a product panel (three plain statements) beside the form; a phone shows the form alone. First-run choices live in the sheet above, not on the lock screen. Typography: Fraunces for names and headings, Instrument Sans for everything else.

**Time on screen.** `lib/format.ts` holds the display zone (`setDisplayZone`); every date and time the UI writes goes through it. The Date and time page names the zone, the online check (switch, status, "Check now", the measured difference to this device's clock) and what is stored (UTC).

**A CSS rule that matters.** The base `button { color: inherit }` reset sits in `@layer base`. Unlayered, it beat every Tailwind v4 utility and made all button text pale; keep every reset in a layer.

## 10. Two themes (2026-11-25)

**Choice.** Settings > Theme (`ThemePage`): *Device setting* (default), *Light*, *Dark*, each with a small drawing of the app in that palette (the first drawn half and half). It applies at once, is kept by the core in `device.json` (the web view stores nothing, D32/D33) and also drives the lock screens, because `device_get` is open while the app lock is closed. On desktop the native window (title bar, dialogs) follows through a hook; on Android the status and navigation bar icons follow through `HexisBars.setLook`.

**How it switches.** `<html data-theme="light|dark">` is written by `lib/theme.ts` once the core has answered; with no attribute the stylesheet follows `prefers-color-scheme`. The palette lives only in `src/styles/tokens.css`: no component carries a hard-coded colour for a surface or a text. The accent has two roles: `brass` (the key button's fill, gold in both themes) and `brass-ink` (the accent as text, line, ring or dot; darker in light so it reaches 4.5:1). `[data-palette="dark|light"]` scopes a palette to a subtree, which the theme previews use.

**Checks.** `src/styles/tokens.test.ts` (vitest) keeps the four copies of the palette identical, requires that every colour the dark palette has is redefined for light, and checks WCAG contrast of every text colour on every surface in both palettes (text 4.5, primary text 7, marks 3), the key button's label in all three states, white text on the danger button, error and accent washes, and that each surface step is a visible step. End to end: `e2e/theme-ui.mjs` (emulated light and dark device against every choice) and `e2e/real-uninstall.mjs` (the choice kept by the core and applied on the next start). Both palettes were also looked at in headless Chromium on the desktop and the phone layout.

**Not verified.** How WebKitGTK and the Android WebView draw the light palette (only headless Chromium was used); the native title bar following the theme; the Android bar icons (the Kotlin was not built here).

## 11. The list, folders and search (2026-11-25)

**What the list pane is now** (`src/components/items/ListPane.tsx`). Title row (the title, an *Add folder* icon button, and *New* on wide layouts); the search box with an *Order and filters* icon button (a small number shows how many filters are on); the tick *Include earlier versions* (it appears while there is a query or the panel is open); the panel; the type chips on narrower layouts; a folder path when a folder is open; then the folders, then the items. A search or any filter looks through **every** folder and shows a flat list, with a line saying so ("3 items found, from every folder"). What the list shows is decided in one place, `visibleList` (`state/app.ts`), so the keyboard shortcuts that step through the list agree with the screen.

* **Folders first.** Folder rows (a round brass-ink icon, the name, "2 items, 1 folder", a chevron) come under a quiet "Folders" heading, then the items under their own group headings. Groups follow the order: time for *Recently changed*, the first letter for the title orders, the type for *Type*, none for the rest (then "Items" is shown only when there are folders above). Rows are as tall as item rows (60 px, 72 px on touch). A folder is not a `data-item-row`, so the arrow keys in the list move between items only.
* **The path.** Under the search: one text button per level, the current one bold and `aria-current="page"`, then a rename and a remove button for the open folder. Removing says what moves up and that nothing is deleted.
* **Where a hit was found.** An item row may carry a third, quiet line (`note`): the folder in a flat list, "Found in Website, Notes" for a deeper hit, "Earlier version 3: Title" for an old version. Never a value.
* **The panel** (`ListTools.tsx`) is inline under the search, not a pop-over, so it works on a phone and needs no positioning: *Order* (a select with the nine orders), type chips (`aria-pressed`), three switches (favorites, has a code, asks for the master password), *Tag* and *Changed* selects, and *At least* / *At most* for the number of versions (a warning when the two cannot both be true), then *Clear filters* and *Done*.
* **Folder dialogs** (`FolderSheets.tsx`, mounted once like the item menu): *Move* (a list of every folder with its depth as indentation, a check on the current one, *New folder* at the bottom), *New folder* / *Rename* (a name and, when inside a folder, "Inside Work"), and *Remove*. The editor's Folder box is the same list as a select, with *New folder…* turning it into a text box (`Work/Clients` nests).
* **Item menu.** *Move to folder* after Archive. A login with several addresses gets one *Open <address>* entry each (the first five) instead of *Launch*.
* **Several websites.** The editor shows one box per address and *Add another website*; the item shows one row per address with its own open and copy buttons.

**Deleting for good** (`TrashPane.tsx`). Step 1 is the warning plus one line about what it will take, from `item_purge_plan` ("Deleted 3 June 2026, less than six months ago. To erase it you need your master password **and** the answer to a sum. From 3 December 2026 the master password alone is enough." or "...over six months ago. Your master password is all it takes."). Step 2 asks for the password and, for a recent item, shows the sum in the same large type as the reset (`prettyEquation`, `aria-label="What is ...?"`) with *Your answer*. A wrong password keeps the sum; a wrong answer says "That is not the answer, so nothing was deleted. Here is a new sum." and shows a new one.

**The copy notice** (`Toasts.tsx`, earlier in this round): a small notice that goes after 5 seconds (`TOAST_MS`) or on a swipe to either side, never a countdown over buttons. **The generator** under the password box has a *Hide* button (the *Generate* button next to the password shows it again).

**Not verified.** How the panel and the folder rows look in a real WebKitGTK window and the Android WebView (headless Chromium only); whether the keyboard comes up on Android when "open in search" focuses the box (a focus without a touch may not show it on every WebView); the screen-reader reading of the path and the panel (roles and labels are set, but no screen reader was run).


## 12. The account bar, the PIN screen and the limited vault (2026-11-25)

* **Account bar** (`AccountBar` in `shell/AccountMenu.tsx`). On a phone every page header ends with two touch targets: a **lock** icon button (closes the vault at once; hidden for the demo vault) and the **account button**, a 32 px avatar in a 40 px (48 px on touch) target. It sits in the header row of the list, Trash, Codes, Settings and every `PageHeader` page, never as a floating button, so it takes no height. A computer shows the account chip in the side bar and draws no bar. While the vault is open on its PIN the avatar carries a small brass key badge.
* **Account sheet** (`AccountSheet`). One short sentence, then the accounts: avatar, name and one quiet line saying what choosing it does ("Open now", "Open now, on its PIN", "Its PIN opens it", "Master password"), a check on the open one. Under the list, only when the vault is limited, a bordered note with *Use master password*. Last row of actions: *Add account*, *Lock now*, *Lock app* (when an app lock is set), *Log out* (tooltip "Locks every account. Nothing is deleted."). Choosing the open account locks it; choosing another one switches at once and the screen that follows asks for that account's PIN (when it waits behind one) or its master password.
* **PIN on the lock screen** (`lock/PinForm.tsx`). When the selected account waits behind a PIN, the lock screen shows one field, *PIN* (hidden, with the show/hide eye), the key button *Open with PIN*, and the line "Secrets and settings will still ask for your master password." Below the rule: *Use the master password instead* (and back), then the account switch. A wrong PIN shakes once and says how many tries are left; the fifth, a timer that ran out or a closed app turns the screen into the ordinary master-password form with one sentence above it saying why.
* **Limited vault.** No banner over the content. The account button's badge, the sheet's note, the chip's "On its PIN" line and the wording of the master-password sheet ("This vault was opened with its PIN, so secrets ask for the master password.") say it where it matters. A change that needs the master password (backup targets, lock timers) opens the same *Confirm it is you* sheet and then runs; cancelling leaves things as they were.
* **Settings > Security > Account PIN.** *Set PIN* / *Change* and *Remove*. The sheet asks for the master password, the new PIN (4 to 64 characters, hidden) and a repeat, with the rules said under the field before the person submits.
