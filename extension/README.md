# The Hexis browser extension

Lets Firefox and Chromium-family browsers on Linux use the passkeys in a Hexis vault. It is one small Manifest V3 extension with a manifest per family. **Read `ai-docs/PASSKEYS.md` §10 first**: the design, what the app decides, what is verified and what is not.

```
src/page.js        the page's own world: replaces navigator.credentials.create / .get for passkey calls (falls back to the browser)
src/content.js     the extension's script in each page: adds the page's address itself, checks the request, talks to the background
src/background.js  asks the native host (dev.habibullah.hexis.passkeys) and passes the answer back; takes the address from the browser's sender
manifest.chromium.json, manifest.firefox.json, icons/
tests/             node --test extension/tests/*.test.mjs   (the three scripts, with stand-ins for the browser)
```

* You do not install this by hand from the repository: Hexis carries the same files and writes them to `<data folder>/extension/{chromium,firefox}` when Settings > Browsers is switched on. `src-tauri/src/bridge/extension.rs` embeds them, so a change here changes what the app writes.
* `python3 scripts/build_extension.py` writes the two store zips to `extension/dist/` (ignored by git) and refuses to run when the sources disagree with what the app writes. `--check` only checks.
* The Chromium manifest's `key` fixes the extension id (`pejbejenceojhjhcanlfnibgdihclnhm`), which is the one origin the native host's manifest allows. A Chrome Web Store item gets an id of its own (the dashboard shows its public key): the steps to settle it, and to sign the Firefox add-on, are in `CONTRIBUTING.md`, "Publishing the browser extension". Neither has been done. The Firefox id is `hexis-passkeys@habibullah.dev`; a release Firefox runs only an add-on signed by Mozilla.
* Whole-chain tests: `e2e/real-bridge.mjs` (Chromium) and `e2e/real-bridge-firefox.mjs` (Firefox, as a temporary add-on); see `e2e/README.md`.
