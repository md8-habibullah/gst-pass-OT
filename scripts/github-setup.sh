#!/usr/bin/env bash
# One-time GitHub repository setup for Hexis. YOU run this; it changes settings on github.com, so neither CI nor an
# assistant runs it. It contains no commit and no push, and it never changes who can see the repository.
#
#   scripts/github-setup.sh plan         print what it would do, change nothing
#   scripts/github-setup.sh apply        make the settings below (asks once before it starts)
#   scripts/github-setup.sh release-notes vX.Y.Z   rewrite the description of an existing release from CHANGELOG.md (after you confirm)
#   scripts/github-setup.sh status       read-only: Pages, latest runs, releases, and the live site (run after you have pushed)
#   scripts/github-setup.sh android-key  create the Android release key and store it in GitHub as secrets (asks first; run it yourself)
#   scripts/github-setup.sh repo-key     print the commands that create the package-repository signing key (apt/dnf) and store it
#   scripts/github-setup.sh publish-release vX.Y.Z   publish a release that exists only as a DRAFT (CI no longer makes drafts: it
#                                                    publishes a new version by itself), after you confirm
#
# Needs the GitHub CLI (`gh`) signed in as the account that owns the repository:  gh auth status
# Order matters (GitHub docs): the Pages source must be "GitHub Actions" BEFORE the first push that contains
# .github/workflows/pages.yml, otherwise a legacy branch deployment can overwrite the site with the raw README.
set -euo pipefail

mode="${1:-plan}"
need() { command -v "$1" > /dev/null || { echo "missing tool: $1" >&2; exit 2; }; }
need gh; need curl
R="${GH_REPO:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"
say() { printf '%s\n' "$*"; }

DESCRIPTION="Hexis: a zero-knowledge, post-quantum password manager and authenticator that syncs to your own S3-compatible storage, one bucket or several. Rust + Tauri, Linux and Android."
# GitHub allows 20 topics at most; apply REPLACES the list, so topics dropped here disappear from the repository.
TOPICS=(hexis password-manager post-quantum zero-knowledge s3 s3-compatible cloud-sync multi-cloud self-hosted backup local-first tauri rust android linux totp authenticator encryption security ml-kem)

case "$mode" in
  plan)
    say "Repository: $R   (account: $(gh api user --jq .login))"
    say "Visibility: $(gh api "repos/$R" --jq .visibility)"
    cat <<EOF

apply would:
  1. set GitHub Pages to deploy from "GitHub Actions"           gh api -X PUT|POST repos/$R/pages -f build_type=workflow
  2. turn on private vulnerability reporting (SECURITY.md)      gh api -X PUT repos/$R/private-vulnerability-reporting
  3. set the description and replace the topics (${#TOPICS[@]} of 20)            gh repo edit, gh api -X PUT repos/$R/topics
  4. turn on secret scanning, push protection and Dependabot security updates (free on public repositories)
  5. once Pages exists, set the repository homepage to the site URL

It would NOT: change visibility, commit, push, create a release, or store any secret.
GitHub Pages sites are public on the internet. On a private repository Pages needs a paid plan, and the repository stays
private while the site is public. If you want the code public too, change that yourself in Settings, General, Danger zone.
EOF
    ;;
  apply)
    say "Repository: $R   (account: $(gh api user --jq .login))"
    read -r -p "Apply the settings listed by 'plan' to $R? [y/N] " ans
    [ "$ans" = "y" ] || [ "$ans" = "Y" ] || { say "Nothing changed."; exit 0; }
    say "-- Pages source"
    if gh api "repos/$R/pages" > /dev/null 2>&1; then
      gh api -X PUT "repos/$R/pages" -f build_type=workflow > /dev/null && say "   Pages already existed: source set to GitHub Actions"
    elif out=$(gh api -X POST "repos/$R/pages" -f build_type=workflow 2>&1); then
      say "   Pages created with source GitHub Actions"
    else
      say "   Could not create Pages: $(printf '%s' "$out" | head -c 300)"
      say "   Most likely your plan does not include Pages for private repositories. Options: upgrade the plan, or make the repository public (your decision, not done here). The workflows still run; only the site is missing."
    fi
    say "-- Private vulnerability reporting"
    if gh api -X PUT "repos/$R/private-vulnerability-reporting" > /dev/null 2>&1; then say "   enabled"
    else say "   not available on this repository or plan (SECURITY.md still describes the report path)"; fi
    say "-- Description and topics"
    gh repo edit "$R" --description "$DESCRIPTION" && say "   description set"
    topics_json=$(printf '%s\n' "${TOPICS[@]}" | python3 -c 'import json,sys; print(json.dumps({"names": [l.strip() for l in sys.stdin if l.strip()]}))')
    if printf '%s' "$topics_json" | gh api -X PUT "repos/$R/topics" --input - > /dev/null; then say "   ${#TOPICS[@]} topics set (the list was replaced)"
    else say "   could not set the topics"; fi
    say "-- Secret scanning and push protection"
    if printf '%s' '{"security_and_analysis":{"secret_scanning":{"status":"enabled"},"secret_scanning_push_protection":{"status":"enabled"}}}' \
        | gh api -X PATCH "repos/$R" --input - > /dev/null 2>&1; then say "   enabled"
    else say "   not available (needs a public repository, or GitHub Advanced Security on a private one)"; fi
    say "-- Dependabot alerts and security updates"
    if gh api -X PUT "repos/$R/vulnerability-alerts" > /dev/null 2>&1 && gh api -X PUT "repos/$R/automated-security-fixes" > /dev/null 2>&1; then say "   enabled"
    else say "   could not be enabled (check Settings, Code security)"; fi
    url=$(gh api "repos/$R/pages" --jq .html_url 2>/dev/null || true)
    if [ -n "$url" ]; then gh repo edit "$R" --homepage "$url" && say "-- Homepage set to $url"; fi
    say
    say "Next (you): commit and push the working tree. The first push starts CI; a push to main also deploys the site and replaces the nightly pre-release."
    say "Optional: scripts/github-setup.sh android-key   (a stable Android signing key for tagged releases)"
    say "Optional: scripts/github-setup.sh repo-key      (the key that signs the apt and dnf repositories behind the one-line installer)"
    say "By hand (GitHub has no API for it): Settings, General, Social preview, upload assets/social-preview.png"
    ;;
  publish-release)
    tag="${2:-}"; case "$tag" in v[0-9]*.[0-9]*.[0-9]*) ;; *) say "usage: scripts/github-setup.sh publish-release vX.Y.Z"; exit 2 ;; esac
    say "Repository: $R   (account: $(gh api user --jq .login))"
    gh release view "$tag" -R "$R" --json name,tagName,isDraft,isPrerelease,url,assets \
      --jq '"title: \(.name)\ntag: \(.tagName)  draft: \(.isDraft)  pre-release: \(.isPrerelease)\nurl: \(.url)\nfiles:\n" + ([.assets[] | "  \(.name)  \(.size) bytes"] | join("\n"))' \
      || { say "No release $tag. CI publishes a release by itself when a push to main raises the version; see CONTRIBUTING.md."; exit 1; }
    read -r -p "Publish $tag (visible to everyone who can see the repository; it also starts the package-repository build)? [y/N] " ans
    [ "$ans" = "y" ] || [ "$ans" = "Y" ] || { say "Nothing changed."; exit 0; }
    gh release edit "$tag" -R "$R" --draft=false --latest && say "Published. Watch the site build: scripts/github-setup.sh status"
    ;;
  release-notes)
    tag="${2:-}"; case "$tag" in v[0-9]*.[0-9]*.[0-9]*) ;; *) say "usage: scripts/github-setup.sh release-notes vX.Y.Z"; exit 2 ;; esac
    say "Repository: $R   (account: $(gh api user --jq .login))"
    site=$(gh api "repos/$R/pages" --jq .html_url 2>/dev/null || true)
    [ -n "$site" ] || { say "This repository has no Pages site yet, so the install commands cannot be written."; exit 1; }
    root="$(cd "$(dirname "$0")/.." && pwd)"
    tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
    gh release download "$tag" -R "$R" -p SHA256SUMS -D "$tmp" > /dev/null 2>&1 || { say "No release $tag with a SHA256SUMS file."; exit 1; }
    cert=$(gh variable get ANDROID_CERT_SHA256 -R "$R" 2> /dev/null || true)
    python3 "$root/scripts/release_notes.py" --version "${tag#v}" --repo "$R" --site "${site%/}" --sums "$tmp/SHA256SUMS" --changelog "$root/CHANGELOG.md" ${cert:+--apk-cert "$cert"} > "$tmp/notes.md"
    say "---- the new description of $tag (from the CHANGELOG.md of this checkout) ----"; cat "$tmp/notes.md"; say "----"
    read -r -p "Replace the description of release $tag with this text? The tag and the files stay as they are. [y/N] " ans
    [ "$ans" = "y" ] || [ "$ans" = "Y" ] || { say "Nothing changed."; exit 0; }
    gh release edit "$tag" -R "$R" --notes-file "$tmp/notes.md" && say "Done: https://github.com/$R/releases/tag/$tag"
    ;;
  status)
    say "== Repository"; gh api "repos/$R" --jq '"visibility: \(.visibility)  default branch: \(.default_branch)  homepage: \(.homepage)"'
    say "== Pages";      gh api "repos/$R/pages" --jq '"source: \(.build_type)  status: \(.status)  url: \(.html_url)  https enforced: \(.https_enforced)"' 2>&1 || true
    say "== Latest workflow runs"; gh run list -R "$R" --limit 6 2>&1 || true
    say "== Releases"; gh release list -R "$R" --limit 5 2>&1 || true
    url=$(gh api "repos/$R/pages" --jq .html_url 2>/dev/null || true)
    if [ -n "$url" ]; then
      say "== Live site checks ($url)"
      for p in "" overview/ download/ docs/architecture/ sitemap.xml robots.txt; do
        printf '  %-22s %s\n' "/${p}" "$(curl -s -o /dev/null -m 15 -w '%{http_code}' "${url%/}/${p}" || echo fail)"
      done
      printf '  %-22s %s (expect 404)\n' "/no-such-page" "$(curl -s -o /dev/null -m 15 -w '%{http_code}' "${url%/}/no-such-page" || echo fail)"
      say "  package repositories and the installer:"
      for p in install.sh hexis-archive-keyring.asc apt/dists/stable/InRelease rpm/x86_64/repodata/repomd.xml arch/x86_64/hexis.db; do
        printf '    %-34s %s\n' "/${p}" "$(curl -s -o /dev/null -m 15 -w '%{http_code}' "${url%/}/${p}" || echo fail)"
      done
      say "  the installer's built-in address (expect this repository's own site):"
      # shellcheck disable=SC2016  # the ${...} is the literal text of the installer line, not an expansion
      curl -s -m 15 "${url%/}/install.sh" | sed -n 's/^SITE="${HEXIS_SITE:-\(.*\)}"$/    \1/p'
      say "  third-party hosts in the home page (expect none besides the repository link):"
      curl -s -m 15 "${url%/}/" | grep -oE '(src|href)="https?://[^"]+"' | sort -u | sed 's/^/    /' || true
    fi
    ;;
  android-key)
    need keytool; need openssl
    ks="${HEXIS_KEYSTORE:-$HOME/hexis-release.jks}"
    say "Repository: $R   (account: $(gh api user --jq .login))"
    cat <<EOF

This creates the ONE key that signs every official Hexis APK, then stores it in GitHub so CI signs releases by itself:
  your backup copy of the key   $ks   (outside the repository, readable only by you)
  encrypted secrets             ANDROID_KEYSTORE_BASE64, ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS
  public repository variable    ANDROID_CERT_SHA256   (the fingerprint, so anyone can check that a download is genuine)

GitHub secrets can never be read back, not even by you. That file and its password are the only copy that exists.
Lose them and phones that installed Hexis can no longer be updated; they would have to uninstall first.
EOF
    if [ -e "$ks" ]; then say "$ks already exists. This will not overwrite a key. Move it away or set HEXIS_KEYSTORE=another-path."; exit 1; fi
    if gh secret list -R "$R" 2> /dev/null | grep -q '^ANDROID_KEYSTORE_BASE64'; then
      say; say "A release key is already stored in $R. Replacing it means every phone with the old key can no longer update."
      read -r -p "Type REPLACE to create a new key anyway: " ans; [ "$ans" = "REPLACE" ] || { say "Nothing changed."; exit 0; }
    fi
    read -r -p "Create the key and store it in $R? [y/N] " ans
    [ "$ans" = "y" ] || [ "$ans" = "Y" ] || { say "Nothing changed."; exit 0; }
    read -r -s -p "Choose a password for the key (at least 12 characters; keep it in your password manager): " pw; echo
    read -r -s -p "Again: " pw2; echo
    [ "$pw" = "$pw2" ] || { say "The two passwords differ. Nothing changed."; exit 1; }
    [ "${#pw}" -ge 12 ] || { say "Too short. Nothing changed."; exit 1; }
    export HEXIS_KS_PASS="$pw"
    (umask 077; keytool -genkeypair -keystore "$ks" -storetype PKCS12 -alias hexis -keyalg RSA -keysize 4096 -validity 10000 \
        -dname "CN=Hexis" -storepass:env HEXIS_KS_PASS -keypass:env HEXIS_KS_PASS > /dev/null 2>&1) || { rm -f "$ks"; say "keytool failed; nothing was stored."; exit 1; }
    chmod 600 "$ks"
    # from the certificate itself, so it does not depend on how this Java version words keytool's listing
    fp=$(keytool -exportcert -keystore "$ks" -alias hexis -storepass:env HEXIS_KS_PASS 2> /dev/null | openssl x509 -inform DER -noout -fingerprint -sha256 2> /dev/null | sed 's/.*=//' | tr -d ':' | tr 'A-F' 'a-f')
    [ "${#fp}" -eq 64 ] || { say "Could not read the key's fingerprint; nothing was stored. The file $ks was created: delete it and try again."; exit 1; }
    base64 "$ks" | tr -d '\n' | gh secret set ANDROID_KEYSTORE_BASE64 -R "$R"
    printf '%s' "$pw" | gh secret set ANDROID_KEYSTORE_PASSWORD -R "$R"
    printf '%s' hexis | gh secret set ANDROID_KEY_ALIAS -R "$R"
    gh variable set ANDROID_CERT_SHA256 -R "$R" --body "$fp"
    unset HEXIS_KS_PASS pw pw2
    cat <<EOF

Done. The key is in GitHub, and CI now signs every APK with it (no "-testkey" in the file name). Certificate fingerprint (SHA-256):
  $fp

Now, by hand:
  1. Back up $ks and its password somewhere that is not this computer (a password manager's file storage, an
     encrypted drive). Never commit it; *.jks is git-ignored.
  2. Nobody else needs the file. When a push to main raises the version, CI signs and publishes the release.
  3. Existing APKs signed with a throw-away key cannot be updated by this key: uninstall them first.
EOF
    ;;
  repo-key)
    cat <<'EOF'
# Create ONE signing key for the apt and rpm repositories that the one-line installer (site/install.sh) registers on
# people's machines. Every machine that installed Hexis from the repository trusts THIS key: keep the backup file
# somewhere safe and never commit it (the file is written outside the repository). If it is lost or leaked, every
# user must be told to import a new key, so treat it like the Android release key.
export GNUPGHOME="$(mktemp -d)"; chmod 700 "$GNUPGHOME"
gpg --batch --passphrase '' --quick-generate-key 'Hexis Releases <releases@hexis.invalid>' rsa4096 sign never
gpg --batch --armor --export-secret-keys > ~/hexis-repo-signing-key.asc && chmod 600 ~/hexis-repo-signing-key.asc
gpg --list-keys --with-fingerprint 'Hexis Releases'      # note the fingerprint; the site prints it on the Download page

# Store it as a repository secret (read by .github/workflows/pages.yml):
gh secret set REPO_SIGNING_KEY < ~/hexis-repo-signing-key.asc
rm -rf "$GNUPGHOME"; unset GNUPGHOME

# The repositories are published when a STABLE release exists. CI publishes one by itself when a push to main raises the
# version, and then starts the Pages workflow, which rebuilds the site with the apt and rpm repositories inside it.
EOF
    ;;
  *) sed -n '2,13p' "$0"; exit 2 ;;
esac
