#!/usr/bin/env node
// Builds the project website (GitHub Pages) from the repository's own Markdown: README, ai-docs/, CONTRIBUTING and friends.
//
//   node scripts/build-site.mjs                 -> site-dist/
//   SITE_URL=https://owner.github.io/repo/ SITE_REPO_URL=https://github.com/owner/repo node scripts/build-site.mjs
//
// Rules this script enforces (it exits 1 when one is broken, so CI catches it before a deploy):
//   * every internal link and every #anchor in every page resolves;
//   * no page loads anything from a third-party host (no scripts, fonts, images, analytics); there is no JavaScript at all;
//   * titles stay under 60 characters and every page has a description.
// Only dependency: `marked` (pinned in package.json). Fonts are the OFL fonts the app already bundles (@fontsource).
import { readFileSync, writeFileSync, mkdirSync, rmSync, existsSync, copyFileSync, statSync, readdirSync } from "node:fs";
import { dirname, join, posix, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { Marked } from "marked";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = resolve(ROOT, process.argv.includes("--out") ? process.argv[process.argv.indexOf("--out") + 1] : "site-dist");
const SITE_URL = (process.env.SITE_URL || "http://localhost:4173/").replace(/\/?$/, "/");
const REPO = (process.env.SITE_REPO_URL || "https://github.com/your-account/passman").replace(/\/$/, "");
const BRANCH = "main";
const errors = [];
const warn = (m) => console.warn("warning:", m);
const fail = (m) => errors.push(m);

const esc = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
const unesc = (s) => s.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, "&");
const read = (p) => readFileSync(join(ROOT, p), "utf8");

// ---------------------------------------------------------------------------------------------------- the page list
// slug "" is the home page. `src` is a repo file rendered as Markdown; `kind` marks the hand-written pages.
const PAGES = [
  { slug: "", kind: "home", title: "Passman", nav: "Home", group: "start",
    desc: "A local-first, zero-knowledge, post-quantum password manager for Linux and Android. No account, no server, no telemetry." },
  { slug: "overview", src: "README.md", title: "Overview", group: "start",
    desc: "What Passman is, what it protects against, how to run it, import and export, and where everything is documented." },
  { slug: "download", kind: "download", title: "Download", group: "start",
    desc: "Linux (deb, rpm, AppImage, tar.gz, Arch) and Android builds, how to verify them, and how to install each." },
  { slug: "docs/architecture", src: "ai-docs/ARCHITECTURE.md", title: "Architecture", group: "docs",
    desc: "Threat model, trust boundaries, the lock state machine, the module map and the decision log." },
  { slug: "docs/crypto-spec", src: "ai-docs/CRYPTO_SPEC.md", title: "Crypto specification", group: "docs",
    desc: "Primitives, key hierarchy, envelope and keyring wire formats, and the test vectors behind them." },
  { slug: "docs/storage-and-sync", src: "ai-docs/STORAGE_AND_SYNC.md", title: "Storage and sync", group: "docs",
    desc: "The SQLite schema, version history, trash, the S3 backup protocol, sync triggers, the encrypted drive and the importer." },
  { slug: "docs/ipc-contract", src: "ai-docs/IPC_CONTRACT.md", title: "IPC contract", group: "docs",
    desc: "Every command the interface can call, with its inputs, outputs and error codes." },
  { slug: "docs/ui-design-system", src: "ai-docs/UI_DESIGN_SYSTEM.md", title: "Design system", group: "docs",
    desc: "The Strongroom design language: colour, type, layouts, motion and accessibility rules." },
  { slug: "docs/build-targets", src: "ai-docs/BUILD_TARGETS.md", title: "Build targets", group: "docs",
    desc: "System packages, Linux bundles, the Android toolchain, the mock S3 server and the exit-flush behaviour." },
  { slug: "docs/feature-queue", src: "ai-docs/FEATURE_QUEUE.md", title: "Feature queue", group: "docs",
    desc: "The queued feature specifications (sync, gate, disguise, drive, importer) and how each was built." },
  { slug: "docs/state", src: "ai-docs/CHANGELOG_AND_STATE.md", title: "Project state", group: "docs",
    desc: "Where the project is right now, what is verified, and the honest list of what is not." },
  { slug: "docs/e2e-tests", src: "e2e/README.md", title: "End-to-end tests", group: "docs",
    desc: "How the real-core end-to-end suites work and how to run them." },
  { slug: "project/contributing", src: "CONTRIBUTING.md", title: "Contributing", group: "project",
    desc: "How to set up, the rules every change follows, and what a good pull request looks like." },
  { slug: "project/security", src: "SECURITY.md", title: "Security policy", group: "project",
    desc: "How to report a vulnerability, what is in scope, and how to verify a download." },
  { slug: "project/privacy", src: "PRIVACY.md", title: "Privacy", group: "project",
    desc: "Passman collects nothing: what the app, its network use and this website do and do not do." },
  { slug: "project/changelog", src: "CHANGELOG.md", title: "Changelog", group: "project",
    desc: "Notable changes by version." },
  { slug: "project/conduct", src: "CODE_OF_CONDUCT.md", title: "Code of conduct", group: "project",
    desc: "How we keep the project calm, honest and respectful." },
  { slug: "project/license", src: "LICENSE", title: "MIT licence", group: "project", plain: true,
    desc: "The MIT licence the project is released under." },
];
const GROUPS = { start: "Start", docs: "Documentation", project: "Project" };
const bySlug = new Map(PAGES.map((p) => [p.slug, p]));
const bySrc = new Map(PAGES.filter((p) => p.src).map((p) => [p.src, p]));
const pageHref = (p) => (p.slug === "" ? "" : `${p.slug}/`);

// ---------------------------------------------------------------------------------------------------- markdown
const githubSlug = (text) => text.trim().toLowerCase().replace(/[^\p{L}\p{N}\s_-]/gu, "").replace(/\s/g, "-");
const links = []; // {from, target, frag}
const idsOf = new Map(); // slug -> Set of heading ids

function renderMarkdown(page) {
  const src = page.src;
  const ids = new Set(); const seen = new Map(); const toc = [];
  idsOf.set(page.slug, ids);
  const resolveHref = (href) => {
    if (/^(https?:)\/\//i.test(href)) return { href, external: true };
    if (/^mailto:/i.test(href)) return { href };
    if (/^[a-z][a-z0-9+.-]*:/i.test(href)) { fail(`${src}: unsupported link scheme in ${href}`); return { href: "#" }; }
    const [rawPath, frag] = href.split("#");
    if (rawPath === "") { links.push({ from: page.slug, target: page.slug, frag }); return { href: `#${frag}` }; }
    const dir = posix.dirname(src);
    const joined = posix.normalize(posix.join(dir, decodeURIComponent(rawPath))).replace(/\/$/, "");
    const target = bySrc.get(joined);
    if (target) {
      links.push({ from: page.slug, target: target.slug, frag });
      return { href: relative(page.slug, target.slug) + (frag ? `#${frag}` : ""), internal: true };
    }
    if (joined.startsWith("../")) return { href: `${REPO}/${joined.replace(/^(\.\.\/)+/, "")}`, external: true };
    const abs = join(ROOT, joined);
    if (existsSync(abs)) {
      const kind = statSync(abs).isDirectory() ? "tree" : "blob";
      return { href: `${REPO}/${kind}/${BRANCH}/${joined}${frag ? `#${frag}` : ""}`, external: true };
    }
    fail(`${src}: link target not found: ${href}`);
    return { href: "#" };
  };
  const marked = new Marked({
    gfm: true,
    renderer: {
      heading({ tokens, depth }) {
        const inner = this.parser.parseInline(tokens);
        const base = githubSlug(unesc(inner.replace(/<[^>]+>/g, "")));
        const n = seen.get(base) ?? 0; seen.set(base, n + 1);
        const id = n === 0 ? base : `${base}-${n}`;
        ids.add(id);
        if (depth === 2) toc.push({ id, text: unesc(inner.replace(/<[^>]+>/g, "")) });
        return `<h${depth} id="${esc(id)}">${inner}</h${depth}>\n`;
      },
      link({ href, title, tokens }) {
        const r = resolveHref(href);
        const rel = r.external ? ' rel="noopener noreferrer"' : "";
        return `<a href="${esc(r.href)}"${title ? ` title="${esc(title)}"` : ""}${rel}>${this.parser.parseInline(tokens)}</a>`;
      },
      // No images: the docs contain none, and a badge or remote picture would be a third-party request.
      image() { return ""; },
      // Raw HTML in the Markdown is shown as text, never interpreted (also keeps `<version>`-style words visible).
      html({ text }) { return esc(text); },
    },
  });
  let html = marked.parse(read(src));
  html = html.replace(/<table>/g, '<div class="table-wrap"><table>').replace(/<\/table>/g, "</table></div>");
  return { html, toc };
}

// ---------------------------------------------------------------------------------------------------- templates
const relative = (fromSlug, toSlug) => {
  const depth = fromSlug === "" ? 0 : fromSlug.split("/").length;
  const up = depth === 0 ? "./" : "../".repeat(depth);
  const to = toSlug === "" ? "" : `${toSlug}/`;
  return (up + to) || "./";
};
const rootOf = (slug) => relative(slug, "");

function sidebar(current) {
  const out = [];
  for (const [g, label] of Object.entries(GROUPS)) {
    out.push(`<h2 class="side-title">${label}</h2><ul>`);
    for (const p of PAGES.filter((x) => x.group === g)) {
      const here = p.slug === current ? ' aria-current="page"' : "";
      out.push(`<li><a href="${relative(current, p.slug)}"${here}>${esc(p.nav || p.title)}</a></li>`);
    }
    out.push("</ul>");
  }
  return out.join("\n");
}

function shell(page, body, { toc = [], wide = false } = {}) {
  const root = rootOf(page.slug);
  const title = page.slug === "" ? "Passman: a password vault that sealed before it syncs" : `${page.title} | Passman`;
  if (title.length > 60) warn(`title longer than 60 characters: ${title}`);
  const url = SITE_URL + pageHref(page);
  const crumbs = page.slug === "" ? [] : [["Passman", SITE_URL], ...(page.slug.includes("/") ? [[GROUPS[page.group], null]] : []), [page.title, url]];
  const ld = page.slug === ""
    ? { "@context": "https://schema.org", "@type": "SoftwareApplication", name: "Passman", applicationCategory: "SecurityApplication",
        operatingSystem: "Linux, Android", description: page.desc, url: SITE_URL, offers: { "@type": "Offer", price: "0", priceCurrency: "USD" } }
    : { "@context": "https://schema.org", "@type": "TechArticle", headline: page.title, description: page.desc, url,
        isPartOf: { "@type": "WebSite", name: "Passman", url: SITE_URL } };
  const breadcrumb = crumbs.length ? { "@context": "https://schema.org", "@type": "BreadcrumbList",
    itemListElement: crumbs.filter((c) => c[1]).map(([name, item], i) => ({ "@type": "ListItem", position: i + 1, name, item })) } : null;
  const tocHtml = toc.length > 3
    ? `<details class="toc"><summary>On this page</summary><ol>${toc.map((t) => `<li><a href="#${esc(t.id)}">${esc(t.text)}</a></li>`).join("")}</ol></details>` : "";
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(title)}</title>
<meta name="description" content="${esc(page.desc)}">
<link rel="canonical" href="${esc(url)}">
<meta name="color-scheme" content="dark light">
<meta property="og:type" content="website">
<meta property="og:site_name" content="Passman">
<meta property="og:title" content="${esc(title)}">
<meta property="og:description" content="${esc(page.desc)}">
<meta property="og:url" content="${esc(url)}">
<meta name="twitter:card" content="summary">
<link rel="icon" href="${root}assets/icon.png" type="image/png">
<link rel="stylesheet" href="${root}assets/site.css">
<script type="application/ld+json">${JSON.stringify(ld)}</script>${breadcrumb ? `\n<script type="application/ld+json">${JSON.stringify(breadcrumb)}</script>` : ""}
</head>
<body>
<a class="skip" href="#main">Skip to the content</a>
<header class="top">
  <a class="mark" href="${root}"><img src="${root}assets/icon.png" alt="" width="28" height="28"><span>Passman</span></a>
  <nav aria-label="Main">
    <a href="${relative(page.slug, "overview")}">Overview</a>
    <a href="${relative(page.slug, "download")}">Download</a>
    <a href="${relative(page.slug, "docs/architecture")}">Docs</a>
    <a href="${relative(page.slug, "project/contributing")}">Contribute</a>
    <a href="${REPO}" rel="noopener noreferrer">Source</a>
  </nav>
</header>
${wide ? `<main id="main" class="wide">${body}</main>` : `<div class="mobile-pages"><details><summary>Pages</summary><nav aria-label="All pages">${sidebar(page.slug)}</nav></details></div>
<div class="layout">
<aside class="side"><nav aria-label="Pages">${sidebar(page.slug)}</nav></aside>
<main id="main" class="doc">
${tocHtml}
<article class="prose">
${body}
</article>
</main>
</div>`}
<footer class="foot">
  <p>Passman is MIT-licensed and pre-1.0; it has not been independently audited. This site sets no cookies and loads nothing from other hosts.</p>
  <p><a href="${relative(page.slug, "project/privacy")}">Privacy</a> · <a href="${relative(page.slug, "project/security")}">Security policy</a> · <a href="${relative(page.slug, "project/license")}">Licence</a> · <a href="${REPO}/issues" rel="noopener noreferrer">Issues</a></p>
</footer>
</body>
</html>
`;
}

// ---------------------------------------------------------------------------------------------------- hand-written pages
function homeBody() {
  const dl = relative("", "download"), arch = relative("", "docs/architecture"), crypto = relative("", "docs/crypto-spec");
  return `
<section class="hero">
  <div class="hero-copy">
    <h1>Sealed on your device, before anything is saved or synced.</h1>
    <p class="lede">Passman keeps passwords, one-time codes, notes and files in a vault only you can open. There is no account and no server of ours: what reaches your storage is ciphertext.</p>
    <p class="actions"><a class="btn primary" href="${dl}">Download Passman</a><a class="btn" href="${arch}">How it is built</a></p>
    <p class="fine">Linux and Android. Pre-1.0, not independently audited.</p>
  </div>
  <figure class="seen" aria-labelledby="seen-cap">
    <figcaption id="seen-cap">What your storage provider sees</figcaption>
<pre class="seen-box"><span class="dim">bucket/passman/</span>
  items/3f9a1c07-5be2-4d10-a7c3-0b6e91d2f4a8   4,212 B
  items/b61e44d2-90ca-47f1-8e2d-71a35c0e9b16   1,980 B
  _keyring                                     1,376 B

<span class="dim">items/3f9a1c07-…</span>
  9f3c 04d7 aa21 e18b 5c70 d9f2 3b14 86ea
  71c5 0be9 4d38 a2f6 c01d 57b3 e9a4 12f0
  <span class="dim">… unreadable without your master password</span></pre>
    <pre class="seen-box you"><span class="dim">what you see in the app</span>
  Mail account
  ana@example.com
  <span class="dim">one-time code</span>  <b>482 913</b></pre>
    <p class="fine">An illustration with made-up values. A provider can see names, sizes and times. It cannot see content.</p>
  </figure>
</section>

<section class="trio">
  <div>
    <h2>Locked by your password, then by a hybrid key</h2>
    <p>Your master password goes through Argon2id. A random vault key is wrapped with ML-KEM-1024 and X25519 together, so a future quantum computer is not enough on its own. Items use XChaCha20-Poly1305. <a href="${crypto}">Read the specification.</a></p>
  </div>
  <div>
    <h2>Offline first, direct to your own storage</h2>
    <p>Everything works without a network. If you add S3-compatible storage, the app talks to it directly from your device, only when you unlock, save a change or leave the app. No timers, no background service, no notifications.</p>
  </div>
  <div>
    <h2>Honest about its limits</h2>
    <p>Malware on an unlocked device, a weak master password and a provider that rolls data back are not fully covered, and the optional PIN gate is a privacy screen, not encryption. The threat model lists every limit. <a href="${arch}">Read it.</a></p>
  </div>
</section>

<section class="list">
  <h2>What is inside</h2>
  <dl>
    <div><dt>Items</dt><dd>Logins, cards, identities, SSH keys, notes and media, with one-time codes, folders, tags and favourites.</dd></div>
    <div><dt>History and trash</dt><dd>Every edit is kept as an immutable version. Deleted items wait in a trash with a two-step permanent delete.</dd></div>
    <div><dt>Several accounts</dt><dd>Independent vaults on one device, each with its own master password and backup targets.</dd></div>
    <div><dt>Encrypted drive</dt><dd>Files up to 50 MB, chunked and authenticated, attachable to items.</dd></div>
    <div><dt>Gate and panic</dt><dd>An optional PIN, password or number grid behind a maths exercise, a panic credential, an attempt limit and a wipe.</dd></div>
    <div><dt>Disguise</dt><dd>Rename and re-icon the app, including its launcher icon on Android.</dd></div>
    <div><dt>Import and export</dt><dd>About 25 formats recognised by content, with a preview before anything is saved; a readable copy on request.</dd></div>
    <div><dt>Backup</dt><dd>One primary S3 target and mirrors, a master backup, and restore into a new vault.</dd></div>
  </dl>
</section>

<section class="list">
  <h2>Built in the open</h2>
  <p class="plain">Every push runs the type checks, unit tests, rustfmt, clippy and the full Rust test suite, then drives the real Rust core from a headless browser, and builds the Linux packages and the Android APK. Pushes to <code>main</code> publish a rolling nightly pre-release; a version tag opens a draft release. <a href="${relative("", "project/contributing")}">Contributing</a> · <a href="${relative("", "docs/state")}">What is verified, and what is not</a></p>
</section>
`;
}

function downloadBody() {
  const rel = `${REPO}/releases`;
  return `
<article class="prose">
<h1>Download Passman</h1>
<p>Builds come from the project's own CI. Get them from the <a href="${rel}" rel="noopener noreferrer">Releases page</a>: a tagged release is the stable choice, and the <a href="${rel}/tag/nightly" rel="noopener noreferrer">nightly pre-release</a> is the newest <code>main</code>. Windows, macOS and iOS are not targets: the code has never been built or tested there.</p>
<div class="table-wrap"><table>
<thead><tr><th>Variant</th><th>File</th><th>Install</th></tr></thead>
<tbody>
<tr><td>Linux, Debian / Ubuntu</td><td><code>Passman_&lt;version&gt;_amd64.deb</code></td><td><code>sudo apt install ./Passman_*.deb</code></td></tr>
<tr><td>Linux, Fedora / openSUSE</td><td><code>Passman-&lt;version&gt;-1.x86_64.rpm</code></td><td><code>sudo dnf install ./Passman-*.rpm</code></td></tr>
<tr><td>Linux, any distribution</td><td><code>Passman_&lt;version&gt;_amd64.AppImage</code></td><td><code>chmod +x Passman_*.AppImage &amp;&amp; ./Passman_*.AppImage</code></td></tr>
<tr><td>Linux, plain archive</td><td><code>passman-&lt;version&gt;-linux-x86_64.tar.gz</code></td><td>unpack, run <code>passman</code>; needs <code>webkit2gtk-4.1</code></td></tr>
<tr><td>Linux, Arch</td><td><a href="${REPO}/blob/${BRANCH}/packaging/arch/PKGBUILD" rel="noopener noreferrer">PKGBUILD</a></td><td><code>makepkg -si</code> in that folder</td></tr>
<tr><td>Android 9+, arm64</td><td><code>passman-&lt;version&gt;-android-arm64.apk</code></td><td>allow installs from your browser or file manager, open the file</td></tr>
</tbody></table></div>
<h2 id="verify-a-download">Verify a download</h2>
<p>Every release lists a <code>SHA256SUMS</code> file. Put it in the same folder as the files you downloaded and run:</p>
<pre><code>sha256sum -c SHA256SUMS --ignore-missing</code></pre>
<p>Every line you downloaded must say <code>OK</code>. An APK with <code>-testkey</code> in its name was signed with a throw-away key made by one CI run: it installs, but cannot be updated in place by a later build. Release APKs are signed with the project's key.</p>
<h2 id="before-you-trust-it-with-anything-important">Before you trust it with anything important</h2>
<p>Passman is pre-1.0 and has had no independent security review. Keep a second copy of anything you cannot lose. The <a href="${relative("download", "docs/state")}">project state page</a> lists what has and has not been verified, including that the Android build has had limited testing on real devices.</p>
</article>
`;
}

// ---------------------------------------------------------------------------------------------------- build
rmSync(OUT, { recursive: true, force: true });
mkdirSync(join(OUT, "assets", "fonts"), { recursive: true });

// fonts (OFL) copied from the packages the app already bundles, with their licences
const FONTS = [
  ["@fontsource-variable/fraunces", "fraunces-latin-wght-normal.woff2", "Fraunces"],
  ["@fontsource-variable/instrument-sans", "instrument-sans-latin-wght-normal.woff2", "Instrument Sans"],
  ["@fontsource/ibm-plex-mono", "ibm-plex-mono-latin-400-normal.woff2", "IBM Plex Mono"],
  ["@fontsource/ibm-plex-mono", "ibm-plex-mono-latin-500-normal.woff2", "IBM Plex Mono"],
];
for (const [pkg, file] of FONTS) {
  const from = join(ROOT, "node_modules", pkg, "files", file);
  if (!existsSync(from)) fail(`font missing: ${pkg}/files/${file} (run npm ci)`); else copyFileSync(from, join(OUT, "assets", "fonts", file));
}
const licenceNote = [...new Set(FONTS.map((f) => f[0]))].map((pkg) => {
  const lic = join(ROOT, "node_modules", pkg, "LICENSE");
  return `== ${pkg}\n${existsSync(lic) ? readFileSync(lic, "utf8") : "(licence file not found)"}`;
}).join("\n\n");
writeFileSync(join(OUT, "assets", "fonts", "FONT-LICENSES.txt"), licenceNote);
copyFileSync(join(ROOT, "src-tauri", "icons", "128x128.png"), join(OUT, "assets", "icon.png"));
copyFileSync(join(ROOT, "site", "site.css"), join(OUT, "assets", "site.css"));

const rendered = new Map();
for (const page of PAGES) {
  let body, toc = [], wide = false;
  if (page.kind === "home") { body = homeBody(); wide = true; }
  else if (page.kind === "download") { body = downloadBody(); }
  else if (page.plain) { body = `<h1>${esc(page.title)}</h1><pre class="licence">${esc(read(page.src))}</pre>`; idsOf.set(page.slug, new Set()); }
  else { ({ html: body, toc } = renderMarkdown(page)); }
  if (page.kind === "download") idsOf.set(page.slug, new Set(["verify-a-download", "before-you-trust-it-with-anything-important"]));
  if (page.kind === "home") idsOf.set(page.slug, new Set());
  rendered.set(page.slug, { body, toc, wide });
}
// the download page links to docs/state from a hand-written page: record it for the checker
links.push({ from: "download", target: "docs/state", frag: undefined });

for (const page of PAGES) {
  const r = rendered.get(page.slug);
  const dir = page.slug === "" ? OUT : join(OUT, page.slug);
  mkdirSync(dir, { recursive: true });
  let html = shell(page, r.body, { toc: r.toc, wide: r.wide });
  // the download page and the home page write their own <article>/<section> wrappers; the markdown pages get one from the shell
  if (page.kind === "download") html = html.replace(/<article class="prose">\n<article class="prose">/, '<article class="prose">').replace(/<\/article>\n<\/article>/, "</article>");
  writeFileSync(join(dir, "index.html"), html);
}

// link and anchor check
for (const l of links) {
  if (!bySlug.has(l.target)) { fail(`${l.from}: link to unknown page ${l.target}`); continue; }
  if (l.frag && !idsOf.get(l.target)?.has(l.frag)) fail(`${l.from}: anchor #${l.frag} does not exist on ${l.target || "home"}`);
}
// no third-party requests in any generated page
const walk = (d) => readdirSync(d, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(d, e.name)) : [join(d, e.name)]));
for (const f of walk(OUT).filter((x) => x.endsWith(".html"))) {
  const html = readFileSync(f, "utf8");
  for (const m of html.matchAll(/<(?:script|img|link|iframe|source|video|audio)\b[^>]*?\b(?:src|href)="(https?:)?\/\/[^"]*"/gi)) {
    if (/rel="canonical"/.test(m[0]) || /<link\b[^>]*rel="canonical"/.test(m[0])) continue;
    fail(`${f.replace(OUT, "")}: loads a remote resource: ${m[0].slice(0, 120)}`);
  }
  if (/<script\b(?![^>]*application\/ld\+json)/i.test(html)) fail(`${f.replace(OUT, "")}: contains a script`);
}

// 404, robots, sitemap, llms.txt, .nojekyll
const page404 = { slug: "404", title: "Page not found", group: "start", desc: "This page does not exist." };
writeFileSync(join(OUT, "404.html"), shell({ ...page404, slug: "" }, `<article class="prose"><h1>Page not found</h1><p>That address does not exist on this site. Start from the <a href="./">home page</a> or the <a href="docs/architecture/">documentation</a>.</p></article>`, { }).replace(/<link rel="canonical"[^>]*>\n/, ""));
writeFileSync(join(OUT, "robots.txt"), `User-agent: *\nAllow: /\n\nSitemap: ${SITE_URL}sitemap.xml\n`);
const today = new Date().toISOString().slice(0, 10);
writeFileSync(join(OUT, "sitemap.xml"), `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${PAGES.map((p) => `  <url><loc>${esc(SITE_URL + pageHref(p))}</loc><lastmod>${today}</lastmod></url>`).join("\n")}\n</urlset>\n`);
writeFileSync(join(OUT, "llms.txt"), `# Passman\n\n> ${bySlug.get("").desc}\n\n${PAGES.filter((p) => p.slug).map((p) => `- [${p.title}](${SITE_URL}${pageHref(p)}): ${p.desc}`).join("\n")}\n`);
writeFileSync(join(OUT, ".nojekyll"), "");

if (errors.length) {
  console.error(`\nsite build FAILED with ${errors.length} problem(s):`);
  for (const e of errors) console.error("  - " + e);
  process.exit(1);
}
console.log(`site built: ${PAGES.length} pages -> ${OUT.replace(ROOT + "/", "")}  (links checked: ${links.length}; SITE_URL=${SITE_URL})`);
