#!/usr/bin/env bash
# One-time GitHub repository setup for Hexis. YOU run this; it changes settings on github.com, so neither CI nor an
# assistant runs it. It contains no commit and no push, and it never changes who can see the repository.
#
#   scripts/github-setup.sh plan         print what it would do, change nothing
#   scripts/github-setup.sh apply        make the settings below (asks once before it starts)
#   scripts/github-setup.sh status       read-only: Pages, latest runs, releases, and the live site (run after you have pushed)
#   scripts/github-setup.sh android-key  print the commands that create the Android release key and store it as secrets
#   scripts/github-setup.sh repo-key     print the commands that create the package-repository signing key (apt/dnf) and store it
#   scripts/github-setup.sh publish-release vX.Y.Z   show the draft release CI made for that tag and, after you confirm, publish it
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

DESCRIPTION="Hexis: a local-first, zero-knowledge, post-quantum password manager and authenticator. Rust + Tauri, Linux and Android, one-command installer."
TOPICS=(hexis password-manager post-quantum zero-knowledge local-first tauri rust react typescript android linux totp authenticator s3 encryption security ml-kem argon2 offline-first privacy)

case "$mode" in
  plan)
    say "Repository: $R   (account: $(gh api user --jq .login))"
    say "Visibility: $(gh api "repos/$R" --jq .visibility)"
    cat <<EOF

apply would:
  1. set GitHub Pages to deploy from "GitHub Actions"           gh api -X PUT|POST repos/$R/pages -f build_type=workflow
  2. turn on private vulnerability reporting (SECURITY.md)      gh api -X PUT repos/$R/private-vulnerability-reporting
  3. set the description and ${#TOPICS[@]} topics                          gh repo edit
  4. once Pages exists, set the repository homepage to the site URL

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
    gh api -X PUT "repos/$R/private-vulnerability-reporting" > /dev/null 2>&1 && say "   enabled" || say "   not available on this repository or plan (SECURITY.md still describes the report path)"
    say "-- Description and topics"
    args=(--description "$DESCRIPTION"); for t in "${TOPICS[@]}"; do args+=(--add-topic "$t"); done
    gh repo edit "$R" "${args[@]}" && say "   done"
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
      || { say "No release $tag yet. Create the tag first; CI then opens a draft."; exit 1; }
    read -r -p "Publish $tag (visible to everyone who can see the repository; it also starts the package-repository build)? [y/N] " ans
    [ "$ans" = "y" ] || [ "$ans" = "Y" ] || { say "Nothing changed."; exit 0; }
    gh release edit "$tag" -R "$R" --draft=false --latest && say "Published. Watch the site build: scripts/github-setup.sh status"
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
      say "  third-party hosts in the home page (expect none besides the repository link):"
      curl -s -m 15 "${url%/}/" | grep -oE '(src|href)="https?://[^"]+"' | sort -u | sed 's/^/    /' || true
    fi
    ;;
  android-key)
    cat <<'EOF'
# Create ONE release key, keep a backup of the file and both passwords somewhere safe (a lost key means no updates
# for installed copies), and never commit it (*.jks and *.keystore are git-ignored).
keytool -genkeypair -v -keystore hexis-release.jks -alias hexis -keyalg RSA -keysize 4096 -validity 10000

# Store it as repository secrets (read by the `android` job in .github/workflows/ci.yml):
base64 -w0 hexis-release.jks | gh secret set ANDROID_KEYSTORE_BASE64
gh secret set ANDROID_KEYSTORE_PASSWORD
gh secret set ANDROID_KEY_ALIAS --body hexis
gh secret set ANDROID_KEY_PASSWORD

# Afterwards, APKs from CI are signed with this key and lose the -testkey suffix.
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

# The repositories are published when a STABLE release exists: tag vX.Y.Z, let CI open the draft release, review it,
# press "Publish release". That event rebuilds the site with the apt and rpm repositories inside it.
EOF
    ;;
  *) sed -n '2,13p' "$0"; exit 2 ;;
esac
