#!/usr/bin/env bash
# Build the signed package repositories that scripts/install.sh registers: an apt repository and an rpm repository,
# plus the public signing key, all as plain static files for GitHub Pages.
#
#   scripts/make-repos.sh --out site-dist --site https://you.github.io/project --deb X.deb --rpm Y.rpm
#
# Signing key: REPO_SIGNING_KEY (the armored private key, no passphrase needed; REPO_SIGNING_PASSPHRASE is optional),
# or a key already present in --gnupghome DIR (then also pass --key FINGERPRINT).
# Tools: gpg, apt-ftparchive (apt-utils), createrepo_c (createrepo-c), rpmsign (rpm), gzip.
#   Ubuntu/Debian:  sudo apt-get install -y apt-utils createrepo-c rpm gnupg
# Output layout under --out:
#   hexis-archive-keyring.gpg   the public key, binary (what apt's Signed-By wants)
#   hexis-archive-keyring.asc   the same key, armored (what rpm/dnf/zypper import)
#   key-fingerprint.txt
#   apt/pool/main/p/hexis/*.deb  apt/dists/stable/{Release,InRelease,Release.gpg,main/binary-amd64/Packages*}
#   rpm/x86_64/*.rpm (signed)  rpm/x86_64/repodata/ (repomd.xml signed)  rpm/hexis.repo
set -euo pipefail

OUT=""; SITE=""; DEB=""; RPM=""; GNUPGHOME_ARG=""; KEY_ARG=""
while [ $# -gt 0 ]; do
    case $1 in
        --out) OUT=$2; shift ;;
        --site) SITE=${2%/}; shift ;;
        --deb) DEB=$2; shift ;;
        --rpm) RPM=$2; shift ;;
        --gnupghome) GNUPGHOME_ARG=$2; shift ;;
        --key) KEY_ARG=$2; shift ;;
        -h|--help) sed -n '2,17p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done
if [ -z "$OUT" ] || [ -z "$SITE" ] || [ ! -f "$DEB" ] || [ ! -f "$RPM" ]; then echo "need --out, --site, --deb FILE and --rpm FILE (see --help)" >&2; exit 2; fi
for t in gpg apt-ftparchive createrepo_c rpmsign gzip; do
    command -v "$t" >/dev/null || { echo "missing tool: $t (sudo apt-get install -y apt-utils createrepo-c rpm gnupg)" >&2; exit 2; }
done

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
if [ -n "$GNUPGHOME_ARG" ]; then
    export GNUPGHOME=$GNUPGHOME_ARG
else
    export GNUPGHOME="$WORK/gnupg"; mkdir -m 700 "$GNUPGHOME"
    [ -n "${REPO_SIGNING_KEY:-}" ] || { echo "REPO_SIGNING_KEY is not set (and no --gnupghome)" >&2; exit 2; }
    printf '%s\n' "$REPO_SIGNING_KEY" | gpg --batch --quiet --import
fi
FPR=${KEY_ARG:-$(gpg --batch --list-secret-keys --with-colons | awk -F: '/^fpr:/ {print $10; exit}')}
[ -n "$FPR" ] || { echo "no secret signing key found" >&2; exit 2; }

PASSFILE=""
GPGSIGN=(gpg --batch --yes --pinentry-mode loopback --local-user "$FPR")
if [ -n "${REPO_SIGNING_PASSPHRASE:-}" ]; then
    PASSFILE="$WORK/pass"; printf '%s' "$REPO_SIGNING_PASSPHRASE" >"$PASSFILE"; chmod 600 "$PASSFILE"
    GPGSIGN+=(--passphrase-file "$PASSFILE")
fi

mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

# ---- public key ---------------------------------------------------------------------------------------------
gpg --batch --yes --export "$FPR" >"$OUT/hexis-archive-keyring.gpg"
gpg --batch --yes --armor --export "$FPR" >"$OUT/hexis-archive-keyring.asc"
printf '%s\n' "$FPR" >"$OUT/key-fingerprint.txt"

# ---- apt ----------------------------------------------------------------------------------------------------
APT="$OUT/apt"; rm -rf "$APT"
mkdir -p "$APT/pool/main/p/hexis" "$APT/dists/stable/main/binary-amd64"
cp "$DEB" "$APT/pool/main/p/hexis/"
(
    cd "$APT"
    apt-ftparchive packages pool >dists/stable/main/binary-amd64/Packages
    gzip -9kf dists/stable/main/binary-amd64/Packages
    apt-ftparchive \
        -o APT::FTPArchive::Release::Origin=Hexis \
        -o APT::FTPArchive::Release::Label=Hexis \
        -o APT::FTPArchive::Release::Suite=stable \
        -o APT::FTPArchive::Release::Codename=stable \
        -o APT::FTPArchive::Release::Architectures=amd64 \
        -o APT::FTPArchive::Release::Components=main \
        -o "APT::FTPArchive::Release::Description=Hexis package repository" \
        release dists/stable >"$WORK/Release"
    cp "$WORK/Release" dists/stable/Release
    "${GPGSIGN[@]}" --clearsign -o dists/stable/InRelease dists/stable/Release
    "${GPGSIGN[@]}" --armor --detach-sign -o dists/stable/Release.gpg dists/stable/Release
)

# ---- rpm ----------------------------------------------------------------------------------------------------
RPMD="$OUT/rpm/x86_64"; rm -rf "$OUT/rpm"; mkdir -p "$RPMD"
cp "$RPM" "$RPMD/"
SIGNCMD="%{__gpg} gpg --batch --no-verbose --no-armor --pinentry-mode loopback"
if [ -n "$PASSFILE" ]; then SIGNCMD="$SIGNCMD --passphrase-file $PASSFILE"; fi
SIGNCMD="$SIGNCMD --no-secmem-warning -u %{_gpg_name} -sbo %{__signature_filename} %{__plaintext_filename}"
for f in "$RPMD"/*.rpm; do
    rpmsign --define "__gpg $(command -v gpg)" --define "_gpg_name $FPR" --define "_gpg_path $GNUPGHOME" --define "__gpg_sign_cmd $SIGNCMD" --addsign "$f"
done
createrepo_c --quiet "$RPMD"
"${GPGSIGN[@]}" --armor --detach-sign -o "$RPMD/repodata/repomd.xml.asc" "$RPMD/repodata/repomd.xml"
cat >"$OUT/rpm/hexis.repo" <<EOF
[hexis]
name=Hexis
baseurl=$SITE/rpm/\$basearch
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=$SITE/hexis-archive-keyring.asc
EOF

echo "repositories written to $OUT (key $FPR)"
find "$OUT" -maxdepth 4 -type f | sed "s#^$OUT/##" | sort
