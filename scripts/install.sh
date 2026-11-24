#!/bin/sh
# Hexis installer for Linux, plus the Android APK download.
# Project: https://github.com/@REPO@   Download page: @SITE@/download/
#
#   curl -fsS @SITE@/install.sh | sh
#   wget -qO- @SITE@/install.sh | sh
#
# Pass options after "sh -s --", for example:  curl -fsS @SITE@/install.sh | sh -s -- --nightly
# Read the options with --help. Read this file before you run it: it is plain POSIX sh.
#
# What it does, in this order:
#   1. Checks the machine (Linux, 64-bit x86, glibc) and finds curl or wget and sudo/doas/run0/pkexec.
#   2. Debian family, Fedora/RHEL family, openSUSE: registers Hexis's signed package repository
#      (a GPG keyring plus one repository file) and installs "hexis" with the system package manager,
#      so later updates arrive with the normal "apt upgrade" / "dnf upgrade" / "zypper update".
#   3. Everything else (Arch, Void, Gentoo, immutable systems, no root): downloads the release tarball or
#      AppImage, checks its SHA-256 against the release's SHA256SUMS, and installs it for the current user.
#   4. Android (Termux) or --apk: downloads the APK and checks it the same way.
# Every command that changes the system is printed before it runs ("+ ..."). Nothing is installed
# before the whole script has arrived (everything is inside main, called on the last line).
# --dry-run shows the plan and changes nothing; --detect shows what was detected and stops.
#
# Later:  --check    tells you whether a newer version exists (changes nothing)
#         --update   brings an installed Hexis up to date, the way it was installed
#         --uninstall removes Hexis AND everything it keeps for your account: vaults, settings, caches (it lists
#                     them and asks first; --keep-data removes only the program)
set -eu

REPO="${HEXIS_REPO:-@REPO@}"
SITE="${HEXIS_SITE:-@SITE@}"
GITHUB="${HEXIS_GITHUB:-https://github.com}"
DEB_PKG="hexis"
RPM_PKG="hexis"
MIN_GLIBC="2.34" # newest glibc symbol the program uses (measured on the CI build, which runs on Ubuntu 22.04)
MIN_GLIBC_APPIMAGE="2.35" # the libraries inside the AppImage
INSTALLER_ID="hexis-install 2"

# ------------------------------------------------------------------------------------------------ small helpers
have() { command -v "$1" >/dev/null 2>&1; }
first_of() { for _c in "$@"; do if have "$_c"; then printf '%s' "$_c"; return 0; fi; done; return 1; }
say() { printf '%s\n' "$*"; }
step() { printf '\n==> %s\n' "$*"; }
warn() { printf 'Warning: %s\n' "$*" >&2; }
die() {
    printf 'Error: %s\n' "$1" >&2
    shift
    for _l in "$@"; do printf '       %s\n' "$_l" >&2; done
    exit 1
}
# Print a command, then run it (or only print it under --dry-run).
show() {
    printf '+ %s\n' "$*" >&2
    if [ "$DRY" = 1 ]; then return 0; fi
    "$@"
}
run_root() {
    if [ -n "$SUDO" ]; then show "$SUDO" "$@"; else show "$@"; fi
}
# a >= b for "major.minor" strings
ver_ge() {
    _am=${1%%.*}; _an=${1#*.}; _an=${_an%%.*}
    _bm=${2%%.*}; _bn=${2#*.}; _bn=${_bn%%.*}
    [ "$_am" -gt "$_bm" ] && return 0
    [ "$_am" -eq "$_bm" ] && [ "$_an" -ge "$_bn" ] && return 0
    return 1
}

usage() {
    cat <<'EOF'
Hexis installer

Usage: install.sh [options]

  --stable            install the latest published release (default)
  --nightly           install the newest build of main (a pre-release, not audited)
  --version X.Y.Z     install that exact version from its release (skips the package repository)
  --apk               only download and verify the Android APK, into the current folder
  --user              install for the current user only, no root (uses the tarball or the AppImage)
  --system            install for the whole machine (needs root, sudo, doas, run0 or pkexec)
  --prefix DIR        install the tarball under DIR (default: /usr/local, or ~/.local with --user)
  --method M          auto (default), repo, deb, rpm, tarball or appimage
  --check             say whether a newer version exists; changes nothing
  --update            bring an installed Hexis up to date, the same way it was installed
  --uninstall         remove Hexis and everything it keeps for your account (vaults, settings, caches, launcher
                      entry). It lists every folder first and asks you to type REMOVE; nothing is kept.
  --keep-data         with --uninstall: remove only the program and leave your data where it is
  --yes               with --uninstall: do not ask (for scripts); the list is still printed
  --dry-run           show what would happen and change nothing
  --detect            show what was detected about this machine and stop
  -h, --help          this text

Environment: HEXIS_CHANNEL=nightly, HEXIS_REPO=owner/name, HEXIS_SITE=https://..., HEXIS_BASE_URL=<folder
holding the release files and SHA256SUMS; used for mirrors and tests>.
EOF
}

# ------------------------------------------------------------------------------------------------ options
parse_args() {
    CHANNEL="${HEXIS_CHANNEL:-stable}"
    VERSION=""; METHOD=auto; MODE=""; PREFIX=""; WANT_APK=0; UNINSTALL=0; UPDATE=0; CHECK=0; DRY=0; DETECT_ONLY=0; KEEP_DATA=0; ASSUME_YES=0
    while [ $# -gt 0 ]; do
        case $1 in
            --stable) CHANNEL=stable ;;
            --nightly) CHANNEL=nightly ;;
            --version) [ $# -ge 2 ] || die "--version needs a value, for example --version 0.1.0"; VERSION=${2#v}; shift ;;
            --version=*) VERSION=${1#--version=}; VERSION=${VERSION#v} ;;
            --apk) WANT_APK=1 ;;
            --user) MODE=user ;;
            --system) MODE=system ;;
            --prefix) [ $# -ge 2 ] || die "--prefix needs a folder"; PREFIX=$2; shift ;;
            --prefix=*) PREFIX=${1#--prefix=} ;;
            --method) [ $# -ge 2 ] || die "--method needs a value"; METHOD=$2; shift ;;
            --method=*) METHOD=${1#--method=} ;;
            --uninstall) UNINSTALL=1 ;;
            --keep-data) KEEP_DATA=1 ;;
            --yes|-y) ASSUME_YES=1 ;;
            --update|--upgrade) UPDATE=1 ;;
            --check) CHECK=1 ;;
            --dry-run) DRY=1 ;;
            --detect) DETECT_ONLY=1 ;;
            -h|--help) usage; exit 0 ;;
            *) usage >&2; die "unknown option: $1" ;;
        esac
        shift
    done
    case $CHANNEL in stable|nightly) ;; *) die "unknown channel '$CHANNEL' (use stable or nightly)" ;; esac
    case $METHOD in auto|repo|deb|rpm|tarball|appimage) ;; *) die "unknown method '$METHOD' (use auto, repo, deb, rpm, tarball or appimage)" ;; esac
    if [ -n "$VERSION" ]; then CHANNEL=version; fi
    if [ $((UNINSTALL + UPDATE + CHECK)) -gt 1 ]; then die "Choose one of --update, --check and --uninstall."; fi
    if [ "$UNINSTALL" = 0 ] && [ $((KEEP_DATA + ASSUME_YES)) -gt 0 ]; then die "--keep-data and --yes belong to --uninstall."; fi
    if [ "$KEEP_DATA" = 1 ] && [ "$ASSUME_YES" = 1 ]; then die "--keep-data and --yes do not go together: with --keep-data there is nothing to confirm."; fi
    case $REPO in *@*) die "this copy of the installer was not built for a project" "set HEXIS_REPO=owner/name and HEXIS_SITE=https://..., or use the copy served by the project site" ;; esac
}

# ------------------------------------------------------------------------------------------------ detection
os_field() { sed -n "s/^$1=//p" "$OSREL" 2>/dev/null | head -n 1 | sed "s/^[\"']//; s/[\"']\$//"; }

detect_platform() {
    KERNEL=$(uname -s 2>/dev/null || echo unknown)
    MACHINE=${HEXIS_ARCH:-$(uname -m 2>/dev/null || echo unknown)}
    case $MACHINE in
        x86_64|amd64) ARCH=x86_64 ;;
        aarch64|arm64|armv8*) ARCH=aarch64 ;;
        *) ARCH=$MACHINE ;;
    esac
    TERMUX=0
    if [ -n "${TERMUX_VERSION:-}" ] || [ "$(uname -o 2>/dev/null || true)" = Android ]; then TERMUX=1; fi

    ROOTFS="${HEXIS_ROOTFS:-}"
    OSREL="$ROOTFS/etc/os-release"
    [ -r "$OSREL" ] || OSREL="$ROOTFS/usr/lib/os-release"
    DISTRO_ID=$(os_field ID); DISTRO_LIKE=$(os_field ID_LIKE); DISTRO_NAME=$(os_field PRETTY_NAME)
    [ -n "$DISTRO_NAME" ] || DISTRO_NAME=${DISTRO_ID:-unknown}
    case " $DISTRO_ID $DISTRO_LIKE " in
        *" nixos "*) FAMILY=nixos ;;
        *" alpine "*) FAMILY=alpine ;;
        *" debian "*|*" ubuntu "*) FAMILY=debian ;;
        *" fedora "*|*" rhel "*|*" centos "*) FAMILY=redhat ;;
        *" suse "*|*" opensuse "*|*" sles "*) FAMILY=suse ;;
        *" arch "*) FAMILY=arch ;;
        *) FAMILY=other ;;
    esac

    # C library: the bundles are built against glibc
    LIBC=unknown; GLIBC_VER=""
    _l=$(ldd --version 2>&1 | head -n 1 || true)
    case $_l in
        *musl*) LIBC=musl ;;
        *[Gg][Ll][Ii][Bb][Cc]*|*"GNU libc"*|*"GNU C Library"*) LIBC=glibc; GLIBC_VER=$(printf '%s' "$_l" | grep -oE '[0-9]+\.[0-9]+$' || true) ;;
    esac

    IMMUTABLE=0
    if [ -e "$ROOTFS/run/ostree-booted" ] || [ "$DISTRO_ID" = steamos ]; then IMMUTABLE=1; fi

    # package manager: the first of these that exists (an immutable system is never changed with its package manager)
    PM=none
    if [ "$IMMUTABLE" = 1 ]; then PM=immutable
    elif have apt-get && have dpkg; then PM=apt
    elif have dnf; then PM=dnf
    elif have zypper; then PM=zypper
    elif have pacman; then PM=pacman
    elif have yum; then PM=yum
    fi

    if [ "$(id -u 2>/dev/null || echo 1000)" = 0 ]; then IS_ROOT=1; SUDO=""; else IS_ROOT=0; SUDO=$(first_of sudo doas run0 pkexec sudo-rs || true); fi
    FETCHER=$(first_of curl wget || true)
    HOST_WEBKIT=0
    if host_has_lib libwebkit2gtk-4.1.so.0 && host_has_lib libgtk-3.so.0; then HOST_WEBKIT=1; fi
}

# Is a shared library installed? Looks in ldconfig's cache first, then in the usual folders.
host_has_lib() {
    if [ -z "$ROOTFS" ]; then
        _ldc=$(first_of ldconfig /sbin/ldconfig /usr/sbin/ldconfig || true)
        if [ -n "$_ldc" ] && "$_ldc" -p 2>/dev/null | grep -q "$1"; then return 0; fi
    fi
    for _d in /usr/lib /usr/lib64 /lib /lib64 /usr/lib/x86_64-linux-gnu /lib/x86_64-linux-gnu /usr/local/lib; do
        if [ -e "$ROOTFS$_d/$1" ]; then return 0; fi
    done
    return 1
}

# Is the package installed (not merely removed with its settings left behind)?
dpkg_has() {
    _st=$(dpkg -s "$1" 2>/dev/null) || return 1
    case $_st in *"Status:"*) case $_st in *"ok installed"*) return 0 ;; *) return 1 ;; esac ;; esac
    return 0
}

print_detect() {
    printf '%-18s %s\n' installer "$INSTALLER_ID" system "$KERNEL $MACHINE" distribution "$DISTRO_NAME (id=${DISTRO_ID:-?}, like=${DISTRO_LIKE:-none}, family=$FAMILY)" \
        libc "$LIBC${GLIBC_VER:+ $GLIBC_VER}" "package manager" "$PM" "immutable system" "$IMMUTABLE" "running as root" "$IS_ROOT" \
        "root helper" "${SUDO:-none}" downloader "${FETCHER:-none}" "webkit2gtk-4.1+gtk3" "$HOST_WEBKIT" termux "$TERMUX"
}

check_supported() {
    if [ "$KERNEL" != Linux ]; then
        die "Hexis has Linux and Android builds only (this is $KERNEL)." "Downloads: $SITE/download/"
    fi
    if [ "$TERMUX" = 1 ] || [ "$WANT_APK" = 1 ]; then return 0; fi
    if [ "$ARCH" != x86_64 ]; then
        die "There is no prebuilt Linux package for $MACHINE yet; the Linux builds are 64-bit x86 only." \
            "On a phone or tablet, use the Android app: re-run with --apk." \
            "To build from source, follow the README: https://github.com/$REPO#run-it-from-source"
    fi
    case $LIBC in
        musl) die "This system uses musl libc (for example Alpine). The Linux builds need glibc." "Build from source: https://github.com/$REPO#run-it-from-source" ;;
    esac
    if [ "$FAMILY" = nixos ]; then
        die "NixOS cannot run generic Linux binaries without extra setup, so this installer stops here." "Build from source: https://github.com/$REPO#run-it-from-source"
    fi
    if [ "$LIBC" = glibc ] && [ -n "$GLIBC_VER" ] && ! ver_ge "$GLIBC_VER" "$MIN_GLIBC"; then
        die "glibc $GLIBC_VER is too old; Hexis needs glibc $MIN_GLIBC or newer (Debian 12, Ubuntu 22.04, Fedora 35 or newer)."
    fi
    if [ -z "$FETCHER" ]; then die "Please install curl or wget first."; fi
}

# ------------------------------------------------------------------------------------------------ network
fetch() { # url dest
    case $FETCHER in
        curl)
            _o="-fsSL --retry 3 --connect-timeout 20"
            case $1 in https://*) _o="$_o --proto =https --tlsv1.2" ;; esac
            # shellcheck disable=SC2086
            curl $_o -o "$2" "$1" ;;
        *) wget -q -O "$2" "$1" ;;
    esac
}
fetch_big() { # url dest: show a progress bar on a terminal
    if [ "$FETCHER" = curl ] && [ -t 2 ]; then
        _o="-fL --retry 3 --connect-timeout 20 -#"
        case $1 in https://*) _o="$_o --proto =https --tlsv1.2" ;; esac
        # shellcheck disable=SC2086
        curl $_o -o "$2" "$1"
    else
        fetch "$1" "$2"
    fi
}
probe() { fetch "$1" "$TMP/probe" 2>/dev/null; }

sha256_of() {
    if have sha256sum; then sha256sum "$1" | cut -d ' ' -f 1
    elif have shasum; then shasum -a 256 "$1" | cut -d ' ' -f 1
    elif have openssl; then openssl dgst -sha256 "$1" | sed 's/^.*= //'
    else die "Cannot check downloads: sha256sum, shasum or openssl is needed."
    fi
}

# ------------------------------------------------------------------------------------------------ release files (direct downloads)
# Sets BASE (folder URL), GH_TAG (for the gh fallback) and downloads SHA256SUMS into $TMP.
resolve_release() {
    USE_GH=0; GH_TAG=""
    if [ -n "${HEXIS_BASE_URL:-}" ]; then BASE=${HEXIS_BASE_URL%/}
    else
        case $CHANNEL in
            nightly) BASE="$GITHUB/$REPO/releases/download/nightly"; GH_TAG=nightly ;;
            version) BASE="$GITHUB/$REPO/releases/download/v$VERSION"; GH_TAG="v$VERSION" ;;
            *) BASE="$GITHUB/$REPO/releases/latest/download" ;;
        esac
    fi
    if fetch "$BASE/SHA256SUMS" "$TMP/SHA256SUMS" 2>/dev/null; then return 0; fi
    # a private repository answers 404 to anonymous downloads; the GitHub CLI can still fetch when it is signed in
    if [ -z "${HEXIS_BASE_URL:-}" ] && have gh && gh auth status >/dev/null 2>&1; then
        if gh release download ${GH_TAG:+"$GH_TAG"} -R "$REPO" -p SHA256SUMS -O "$TMP/SHA256SUMS" --clobber >/dev/null 2>&1; then USE_GH=1; return 0; fi
    fi
    if [ "$CHANNEL" = stable ] && [ -z "${HEXIS_BASE_URL:-}" ]; then
        warn "No stable release is published yet; using the nightly build instead (a pre-release, not audited)."
        CHANNEL=nightly; resolve_release; return 0
    fi
    die "Could not download the release list from $BASE/SHA256SUMS" \
        "Either that release does not exist, you are offline, or the repository is private (anonymous downloads then fail;" \
        "sign in with the GitHub CLI: gh auth login, and run this again)."
}

# pick PATTERN -> sets PICK_NAME and PICK_SUM from SHA256SUMS
pick() {
    PICK_NAME=""; PICK_SUM=""
    while read -r _h _n; do
        _n=${_n#\*}
        # shellcheck disable=SC2254
        case $_n in $1) PICK_NAME=$_n; PICK_SUM=$_h; return 0 ;; esac
    done <"$TMP/SHA256SUMS"
    return 1
}

# download_verified NAME SUM DEST: download, check the SHA-256, refuse on mismatch
download_verified() {
    if [ "$DRY" = 1 ]; then say "would download $1 and check its SHA-256 ($2)"; return 0; fi
    say "Downloading $1"
    if [ "$USE_GH" = 1 ]; then
        gh release download ${GH_TAG:+"$GH_TAG"} -R "$REPO" -p "$1" -O "$3" --clobber >/dev/null || die "download failed: $1"
    else
        fetch_big "$BASE/$1" "$3" || die "download failed: $BASE/$1"
    fi
    _got=$(sha256_of "$3")
    if [ "$_got" != "$2" ]; then
        rm -f "$3"
        die "CHECKSUM MISMATCH for $1" "expected $2" "got      $_got" "The file was deleted. Nothing was installed. Try again; if it repeats, do not install it."
    fi
    say "SHA-256 matches the release list."
}

# ------------------------------------------------------------------------------------------------ package repositories
repo_available() { # sets REPO_KIND for the package manager
    case $PM in
        apt) probe "$SITE/apt/dists/stable/InRelease" ;;
        dnf|yum|zypper) probe "$SITE/rpm/x86_64/repodata/repomd.xml" ;;
        pacman) probe "$SITE/arch/x86_64/hexis.db" ;;
        *) return 1 ;;
    esac
}

install_from_repo() {
    case $PM in
        apt)
            step "Registering the Hexis apt repository ($SITE/apt)"
            export DEBIAN_FRONTEND=noninteractive
            fetch "$SITE/hexis-archive-keyring.gpg" "$TMP/keyring.gpg" || die "could not download the signing key from $SITE"
            cat >"$TMP/hexis.sources" <<EOF
Types: deb
URIs: $SITE/apt
Suites: stable
Components: main
Architectures: amd64
Signed-By: /usr/share/keyrings/hexis-archive-keyring.gpg
EOF
            run_root install -Dm644 "$TMP/keyring.gpg" /usr/share/keyrings/hexis-archive-keyring.gpg
            run_root install -Dm644 "$TMP/hexis.sources" /etc/apt/sources.list.d/hexis.sources
            run_root apt-get update || die 'The "apt-get update" command is not working on this system, so the installer cannot go on.' "Fix the apt configuration and run this again."
            if [ "$UPDATE" = 1 ]; then run_root apt-get install -y --only-upgrade "$DEB_PKG"; else run_root apt-get install -y "$DEB_PKG"; fi
            ;;
        dnf|yum)
            step "Registering the Hexis rpm repository ($SITE/rpm)"
            write_rpm_repo_file
            run_root install -Dm644 "$TMP/hexis.repo" /etc/yum.repos.d/hexis.repo
            if [ "$UPDATE" = 1 ]; then
                run_root "$PM" clean expire-cache # a cached repository index would hide the new version
                run_root "$PM" upgrade -y "$RPM_PKG"
            else run_root "$PM" install -y "$RPM_PKG"; fi
            ;;
        zypper)
            step "Registering the Hexis rpm repository ($SITE/rpm)"
            write_rpm_repo_file
            run_root install -Dm644 "$TMP/hexis.repo" /etc/zypp/repos.d/hexis.repo
            run_root zypper --non-interactive --gpg-auto-import-keys refresh hexis
            if [ "$UPDATE" = 1 ]; then run_root zypper --non-interactive update "$RPM_PKG"; else run_root zypper --non-interactive install "$RPM_PKG"; fi
            ;;
        pacman)
            step "Registering the Hexis pacman repository ($SITE/arch)"
            fetch "$SITE/hexis-archive-keyring.asc" "$TMP/key.asc" || die "could not download the signing key from $SITE"
            fetch "$SITE/key-fingerprint.txt" "$TMP/fpr" || die "could not download the key fingerprint from $SITE"
            _fpr=$(head -n 1 "$TMP/fpr" | tr -d ' \r')
            case $_fpr in *[!0-9A-Fa-f]*|"") die "the key fingerprint file is not a fingerprint" ;; esac
            if [ "${#_fpr}" -ne 40 ]; then die "the key fingerprint file is not a 40-digit fingerprint"; fi
            run_root pacman-key --add "$TMP/key.asc"
            run_root pacman-key --lsign-key "$_fpr"
            if ! grep -q '^\[hexis\]' "$ROOTFS/etc/pacman.conf" 2>/dev/null; then
                cat >"$TMP/pacman.block" <<EOF

# hexis-repo-begin key=$_fpr
[hexis]
SigLevel = Required DatabaseRequired
Server = $SITE/arch/\$arch
# hexis-repo-end
EOF
                run_root tee -a /etc/pacman.conf <"$TMP/pacman.block" >/dev/null
            fi
            # fetch only this repository's database: refreshing every repository (pacman -Sy) before installing one
            # package is the "partial upgrade" Arch warns about, so the other databases are left exactly as they are
            _sync=$(pacman-conf DBPath 2>/dev/null || echo /var/lib/pacman/)
            _sync="${_sync%/}/sync"
            fetch "$SITE/arch/x86_64/hexis.db" "$TMP/hexis.db" || die "could not download the repository database"
            fetch "$SITE/arch/x86_64/hexis.db.sig" "$TMP/hexis.db.sig" || die "could not download the repository database signature"
            run_root install -Dm644 "$TMP/hexis.db" "$_sync/hexis.db"
            run_root install -Dm644 "$TMP/hexis.db.sig" "$_sync/hexis.db.sig"
            run_root pacman -S --needed --noconfirm "$DEB_PKG" || die "pacman could not install hexis." \
                "If it says a library could not be found or downloaded, update the system first (sudo pacman -Syu), then run this again."
            ;;
    esac
}

write_rpm_repo_file() {
    cat >"$TMP/hexis.repo" <<EOF
[hexis]
name=Hexis
baseurl=$SITE/rpm/\$basearch
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=$SITE/hexis-archive-keyring.asc
EOF
}

# ------------------------------------------------------------------------------------------------ direct installs
install_deb() {
    pick 'Hexis_*_amd64.deb' || die "No .deb in this release."
    download_verified "$PICK_NAME" "$PICK_SUM" "$TMP/$PICK_NAME"
    chmod 644 "$TMP/$PICK_NAME" 2>/dev/null || true
    chmod 755 "$TMP"
    step "Installing $PICK_NAME with apt (it fetches the libraries it needs)"
    export DEBIAN_FRONTEND=noninteractive
    run_root apt-get update || warn "apt-get update failed; trying the install anyway."
    run_root apt-get install -y "$TMP/$PICK_NAME"
}

install_rpm() {
    pick 'Hexis-*.x86_64.rpm' || die "No .rpm in this release."
    download_verified "$PICK_NAME" "$PICK_SUM" "$TMP/$PICK_NAME"
    chmod 644 "$TMP/$PICK_NAME" 2>/dev/null || true
    step "Installing $PICK_NAME with $PM (it fetches the libraries it needs)"
    case $PM in
        zypper) run_root zypper --non-interactive install --allow-unsigned-rpm "$TMP/$PICK_NAME" ;;
        *) run_root "$PM" install -y "$TMP/$PICK_NAME" ;;
    esac
}

# where files go for tarball / AppImage installs
set_paths() {
    if [ -n "$PREFIX" ]; then P_ROOT=$PREFIX
    elif [ "$MODE" = user ]; then P_ROOT="$HOME/.local"
    else P_ROOT=/usr/local; fi
    P_BIN="$P_ROOT/bin"
    NEED_ROOT=0
    if [ "$IS_ROOT" = 0 ]; then
        _probe=$P_ROOT
        while [ ! -d "$_probe" ] && [ "$_probe" != / ]; do _probe=$(dirname "$_probe"); done
        if [ ! -w "$_probe" ]; then NEED_ROOT=1; fi
    fi
    if [ "$NEED_ROOT" = 1 ] && { [ "$MODE" = user ] || [ -z "$SUDO" ]; }; then
        die "Cannot write to $P_ROOT." "Choose a folder you own with --prefix, or use --user (installs under ~/.local)."
    fi
}

# Describe the staged tree in $S: every file is listed so --uninstall removes exactly these and nothing else.
write_manifest() { # method
    mkdir -p "$S/share/hexis"
    ( cd "$S" && find . -path ./share/hexis/app -prune -o \( -type f -o -type l \) -print | sed 's#^\./##' | sort ) >"$TMP/manifest.files"
    {
        printf '# installed by %s\n' "$INSTALLER_ID"
        printf 'method=%s\nversion=%s\nroot=%s\nsha256=%s\n' "$1" "$INSTALLED_VERSION" "$P_ROOT" "$INSTALLED_SHA"
        sed 's#^#file=#' "$TMP/manifest.files"
        printf 'file=share/hexis/install-manifest\n'
        if [ -d "$S/share/hexis/app" ]; then printf 'dir=share/hexis/app\n'; fi
    } >"$S/share/hexis/install-manifest"
}
# An update must not leave files behind that the new version no longer has: everything the previous manifest lists
# and the new manifest does not is removed, and a previous unpacked AppImage tree is replaced as a whole.
remove_stale_files() {
    _old="$P_ROOT/share/hexis/install-manifest"
    [ -f "$_old" ] || return 0
    while IFS= read -r _l; do
        case $_l in
            file=*)
                _f=${_l#file=}
                if ! grep -qxF "file=$_f" "$S/share/hexis/install-manifest"; then
                    if [ "$NEED_ROOT" = 1 ]; then run_root rm -f "$P_ROOT/$_f"; else show rm -f "$P_ROOT/$_f"; fi
                fi ;;
            dir=*)
                if [ "$NEED_ROOT" = 1 ]; then run_root rm -rf "$P_ROOT/${_l#dir=}"; else show rm -rf "$P_ROOT/${_l#dir=}"; fi ;;
        esac
    done <"$_old"
}

copy_stage() {
    step "Installing into $P_ROOT"
    remove_stale_files
    if [ "$NEED_ROOT" = 1 ]; then
        run_root mkdir -p "$P_ROOT"
        run_root cp -R "$S/." "$P_ROOT/"
    else
        show mkdir -p "$P_ROOT"
        show cp -R "$S/." "$P_ROOT/"
    fi
}

refresh_desktop() {
    if [ "$DRY" = 1 ]; then return 0; fi
    if have update-desktop-database; then
        if [ "$NEED_ROOT" = 1 ]; then $SUDO update-desktop-database "$P_ROOT/share/applications" >/dev/null 2>&1 || true
        else update-desktop-database "$P_ROOT/share/applications" >/dev/null 2>&1 || true; fi
    fi
    if have gtk-update-icon-cache; then
        if [ "$NEED_ROOT" = 1 ]; then $SUDO gtk-update-icon-cache -q -t "$P_ROOT/share/icons/hicolor" >/dev/null 2>&1 || true
        else gtk-update-icon-cache -q -t "$P_ROOT/share/icons/hicolor" >/dev/null 2>&1 || true; fi
    fi
}

stage_dirs() {
    S="$TMP/stage"
    mkdir -p "$S/bin" "$S/share/applications" "$S/share/icons/hicolor/128x128/apps" "$S/share/doc/hexis" "$S/share/hexis"
}
write_desktop_entry() {
    cat >"$S/share/applications/hexis.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Hexis
Comment=Post-quantum zero-knowledge password vault
Exec=$P_BIN/hexis
Icon=hexis
Terminal=false
Categories=Utility;Security;
StartupWMClass=hexis
EOF
}

# Installs that no package manager looks after get a short command of their own: hexis-update runs this installer
# from the project site with --update (the same script, the same checks). Package installs update with apt, dnf ... .
write_update_command() {
    cat >"$S/bin/hexis-update" <<EOF
#!/bin/sh
# Brings Hexis up to date. This is the same as: curl -fsS $SITE/install.sh | sh -s -- --update
# Read the installer first at $SITE/download/install-script/
if command -v curl >/dev/null 2>&1; then curl -fsS "$SITE/install.sh" | sh -s -- --update "\$@"
elif command -v wget >/dev/null 2>&1; then wget -qO- "$SITE/install.sh" | sh -s -- --update "\$@"
else echo "hexis-update needs curl or wget." >&2; exit 1; fi
EOF
    chmod 755 "$S/bin/hexis-update"
}

install_tarball() {
    pick 'hexis-*-linux-x86_64.tar.gz' || die "No tarball in this release."
    download_verified "$PICK_NAME" "$PICK_SUM" "$TMP/$PICK_NAME"
    INSTALLED_VERSION=${PICK_NAME#hexis-}; INSTALLED_VERSION=${INSTALLED_VERSION%-linux-x86_64.tar.gz}; INSTALLED_SHA=$PICK_SUM
    if [ "$DRY" = 1 ]; then say "would unpack it and install into $P_ROOT (bin, share/applications, share/icons, share/hexis)"; return 0; fi
    mkdir -p "$TMP/unpack"
    tar -xzf "$TMP/$PICK_NAME" -C "$TMP/unpack"
    _src=$(find "$TMP/unpack" -maxdepth 1 -mindepth 1 -type d | head -n 1)
    if [ ! -f "$_src/hexis" ]; then die "The archive does not contain the hexis program."; fi
    stage_dirs
    cp "$_src/hexis" "$S/bin/hexis"; chmod 755 "$S/bin/hexis"
    cp "$_src/hexis.png" "$S/share/icons/hicolor/128x128/apps/hexis.png"
    if [ -f "$_src/LICENSE" ]; then cp "$_src/LICENSE" "$S/share/doc/hexis/LICENSE"; fi
    write_desktop_entry
    write_update_command
    write_manifest tarball
    copy_stage
    refresh_desktop
}

install_tarball_deps() { # system installs only: bring in the two libraries the program links against
    [ "$HOST_WEBKIT" = 1 ] && return 0
    step "Installing the libraries Hexis needs (WebKitGTK 4.1 and GTK 3)"
    case $PM in
        apt) run_root apt-get install -y libwebkit2gtk-4.1-0 libgtk-3-0 ;;
        dnf|yum) run_root "$PM" install -y webkit2gtk4.1 gtk3 ;;
        zypper) run_root zypper --non-interactive install libwebkit2gtk-4_1-0 libgtk-3-0 ;;
        pacman) run_root pacman -S --needed --noconfirm webkit2gtk-4.1 gtk3 || die "pacman could not install the libraries." "Update the system first (sudo pacman -Syu), then run this again." ;;
        *) die "This system's libraries cannot be installed automatically." "Install WebKitGTK 4.1 and GTK 3 with your package manager, then run this again, or use --method appimage." ;;
    esac
}

install_appimage() {
    if [ -n "$GLIBC_VER" ] && ! ver_ge "$GLIBC_VER" "$MIN_GLIBC_APPIMAGE"; then
        die "The AppImage needs glibc $MIN_GLIBC_APPIMAGE or newer (this system has $GLIBC_VER)."
    fi
    pick 'Hexis_*_amd64.AppImage' || die "No AppImage in this release."
    download_verified "$PICK_NAME" "$PICK_SUM" "$TMP/$PICK_NAME"
    INSTALLED_VERSION=${PICK_NAME#Hexis_}; INSTALLED_VERSION=${INSTALLED_VERSION%_amd64.AppImage}; INSTALLED_SHA=$PICK_SUM
    if [ "$DRY" = 1 ]; then say "would install the AppImage under $P_ROOT/share/hexis and a launcher at $P_BIN/hexis"; return 0; fi
    chmod 755 "$TMP/$PICK_NAME"
    stage_dirs
    if host_has_lib libfuse.so.2; then
        # FUSE 2 is present: keep the single file
        cp "$TMP/$PICK_NAME" "$S/share/hexis/Hexis.AppImage"
        printf '#!/bin/sh\nexec "%s/share/hexis/Hexis.AppImage" "$@"\n' "$P_ROOT" >"$S/bin/hexis"
        mkdir -p "$TMP/ex"
        ( cd "$TMP/ex" && "$TMP/$PICK_NAME" --appimage-extract hexis.png >/dev/null 2>&1 ) || true
        if [ -f "$TMP/ex/squashfs-root/hexis.png" ]; then cp "$TMP/ex/squashfs-root/hexis.png" "$S/share/icons/hicolor/128x128/apps/hexis.png"; fi
    else
        # no FUSE: unpack once and run the unpacked tree (starts faster than extracting on every launch)
        mkdir -p "$TMP/ex"
        ( cd "$TMP/ex" && "$TMP/$PICK_NAME" --appimage-extract >/dev/null 2>&1 ) || die "Could not unpack the AppImage."
        mv "$TMP/ex/squashfs-root" "$S/share/hexis/app"
        printf '#!/bin/sh\nexec "%s/share/hexis/app/AppRun" "$@"\n' "$P_ROOT" >"$S/bin/hexis"
        if [ -f "$S/share/hexis/app/hexis.png" ]; then cp "$S/share/hexis/app/hexis.png" "$S/share/icons/hicolor/128x128/apps/hexis.png"; fi
    fi
    chmod 755 "$S/bin/hexis"
    write_desktop_entry
    write_update_command
    write_manifest appimage
    copy_stage
    refresh_desktop
}

install_apk() {
    resolve_release
    pick 'hexis-*-android-arm64*.apk' || die "No Android APK in this release."
    _dest=${PWD:-.}
    if [ "$TERMUX" = 1 ] && [ -d "$HOME/storage/downloads" ]; then _dest="$HOME/storage/downloads"; fi
    download_verified "$PICK_NAME" "$PICK_SUM" "$_dest/$PICK_NAME"
    say ""
    say "Saved: $_dest/$PICK_NAME"
    case $PICK_NAME in *-testkey*) say "Note: '-testkey' means a throw-away signing key. It installs, but a later build cannot update it in place." ;; esac
    if [ "$TERMUX" = 1 ]; then
        say "Open it with:  termux-open \"$_dest/$PICK_NAME\"   (allow installs from this app when Android asks)"
        [ -d "$HOME/storage/downloads" ] || say "Tip: run termux-setup-storage once so the file lands in your Downloads folder."
    else
        say "Install it on a phone with USB debugging:  adb install \"$_dest/$PICK_NAME\""
        say "Or copy the file to the phone and open it there (allow installs from your file manager)."
    fi
}

# ------------------------------------------------------------------------------------------------ what is installed, check, update
# Sets PKG_VER and PKG_KIND for a package of the system package manager: "repo" when the signed repository this
# installer registers is still registered (the package manager then does the updating), "deb" or "rpm" for a package
# that came from a downloaded file. Both stay empty when there is no such package.
pkg_probe() {
    PKG_VER=""; PKG_KIND=""
    if have dpkg && dpkg_has "$DEB_PKG"; then
        PKG_KIND=deb
        # shellcheck disable=SC2016
        PKG_VER=$(dpkg-query -W -f '${Version}' "$DEB_PKG" 2>/dev/null | sed 's/-[^-]*$//' || true)
        if [ -e "$ROOTFS/etc/apt/sources.list.d/hexis.sources" ]; then PKG_KIND=repo; fi
    elif have rpm && rpm -q "$RPM_PKG" >/dev/null 2>&1; then
        PKG_KIND=rpm
        PKG_VER=$(rpm -q --qf '%{VERSION}' "$RPM_PKG" 2>/dev/null || true)
        if [ -e "$ROOTFS/etc/yum.repos.d/hexis.repo" ] || [ -e "$ROOTFS/etc/zypp/repos.d/hexis.repo" ]; then PKG_KIND=repo; fi
    elif have pacman && pacman -Q "$DEB_PKG" >/dev/null 2>&1; then
        PKG_KIND=repo # the only way this installer puts a pacman package on a system is through its repository
        PKG_VER=$(pacman -Q "$DEB_PKG" 2>/dev/null | sed 's/^[^ ]* //; s/-[0-9][0-9]*$//' || true)
    fi
}

# One line per install found, in $TMP/installs:  kind|version|root|sha256
# kind is repo, deb, rpm (system packages) or tarball, appimage (files under a folder, described by a manifest).
detect_installs() {
    : >"$TMP/installs"
    pkg_probe
    if [ -n "$PKG_KIND" ]; then printf '%s|%s||\n' "$PKG_KIND" "${PKG_VER:-unknown}" >>"$TMP/installs"; fi
    for _root in "${PREFIX:-}" "$HOME/.local" /usr/local; do
        [ -n "$_root" ] || continue
        _m="$_root/share/hexis/install-manifest"
        [ -f "$_m" ] || continue
        if grep -qF "|$_root|" "$TMP/installs"; then continue; fi
        _method=$(sed -n 's/^method=//p' "$_m" | head -n 1)
        _ver=$(sed -n 's/^version=//p' "$_m" | head -n 1)
        _sum=$(sed -n 's/^sha256=//p' "$_m" | head -n 1)
        printf '%s|%s|%s|%s\n' "${_method:-unknown}" "${_ver:-unknown}" "$_root" "$_sum" >>"$TMP/installs"
    done
}

describe_install() { # kind root
    case $1 in
        repo) say "the system package, from the signed Hexis repository ($PM keeps it up to date)" ;;
        deb) say "a .deb package that was installed from a downloaded file" ;;
        rpm) say "an .rpm package that was installed from a downloaded file" ;;
        tarball) say "the tarball, files under $2" ;;
        appimage) say "the AppImage, files under $2" ;;
        *) say "installed under ${2:-the system}" ;;
    esac
}

# Reads the newest release's list and sets LATEST_VER and LATEST_SUM (the tarball's checksum).
latest_release() {
    LATEST_VER=""; LATEST_SUM=""
    resolve_release
    if pick 'hexis-*-linux-x86_64.tar.gz'; then
        LATEST_VER=${PICK_NAME#hexis-}; LATEST_VER=${LATEST_VER%-linux-x86_64.tar.gz}; LATEST_SUM=$PICK_SUM
    fi
}

# is_current VERSION SHA256: does the install match the newest release? The nightly keeps one version number,
# so only a checksum can tell its builds apart; an install without one is treated as "maybe older".
is_current() {
    [ -n "$LATEST_VER" ] && [ "$1" = "$LATEST_VER" ] || return 1
    if [ "$CHANNEL" = nightly ]; then [ -n "$2" ] && [ "$2" = "$LATEST_SUM" ] || return 1; fi
    return 0
}

do_check() {
    detect_installs
    if [ ! -s "$TMP/installs" ]; then
        say "Hexis is not installed by this installer."
        say "Install it with:  curl -fsS $SITE/install.sh | sh"
        return 0
    fi
    latest_release
    case $CHANNEL in nightly) _label="nightly build" ;; version) _label="release v$VERSION" ;; *) _label="stable release" ;; esac
    say "Newest $_label: ${LATEST_VER:-unknown}"
    _stale=0
    while IFS='|' read -r _kind _ver _root _sum <&3; do
        say ""
        say "Installed: $_ver, $(describe_install "$_kind" "$_root")"
        if is_current "$_ver" "$_sum"; then say "  Up to date."
        elif [ -n "$LATEST_VER" ] && [ "$_ver" != "$LATEST_VER" ] && [ "$_ver" != unknown ] && ver_ge "$_ver" "$LATEST_VER"; then
            say "  This is newer than the newest published $_label (a development build?)."
        elif [ "$CHANNEL" = nightly ] && [ -z "$_sum" ]; then
            say "  The nightly keeps one version number; run --update --nightly to be sure you have the newest build."; _stale=1
        else say "  An update is available."; _stale=1; fi
    done 3<"$TMP/installs"
    if [ "$_stale" = 1 ]; then
        say ""
        _ch=""
        if [ "$CHANNEL" = nightly ]; then _ch=" --nightly"; fi
        say "Update with:  curl -fsS $SITE/install.sh | sh -s -- --update$_ch"
        if grep -q '^repo|' "$TMP/installs"; then
            case $PM in
                apt) say "or with apt:    sudo apt-get update && sudo apt-get install --only-upgrade $DEB_PKG" ;;
                dnf|yum) say "or with $PM:    sudo $PM upgrade --refresh $RPM_PKG" ;;
                zypper) say "or with zypper: sudo zypper refresh && sudo zypper update $RPM_PKG" ;;
                pacman) say "or with pacman: re-run the command above (it refreshes only the Hexis repository)" ;;
            esac
        fi
    fi
}

# Update one install the way it was made: a repository install through the repository, a downloaded package through
# the same package manager, files under a folder in the same folder. Nothing switches method behind your back.
update_one() { # kind version root sha256
    _kind=$1; _ver=$2; _root=$3; _sum=$4
    PLAN=""; MODE=system
    case $_kind in
        repo)
            case $PM in apt|dnf|yum|zypper|pacman) ;; *) die "This system's package manager ($PM) cannot update the Hexis repository package." ;; esac
            PLAN=repo ;;
        deb) PLAN=deb ;;
        rpm) PLAN=rpm ;;
        tarball|appimage)
            PLAN=$_kind; PREFIX=$_root
            if [ "$_root" = "$HOME/.local" ]; then MODE=user; fi ;;
        *) warn "The install under ${_root:-the system} was made by an unknown method ($_kind); skipping it."; return 0 ;;
    esac
    if [ "$MODE" = system ] && [ "$IS_ROOT" = 0 ] && [ -z "$SUDO" ]; then
        die "Updating a system-wide install needs root, and no sudo, doas, run0 or pkexec was found."
    fi
    if [ "$PLAN" = repo ]; then
        repo_available || die "The Hexis package repository at $SITE cannot be reached, so the update stops here." "Check the network, or use your package manager directly."
    else
        latest_release
        if is_current "$_ver" "$_sum"; then say "Already up to date ($_ver)."; return 0; fi
        if [ "$CHANNEL" = nightly ] && [ -z "$_sum" ]; then say "Nightly: reinstalling to get the newest build."; fi
    fi
    set_paths
    run_plan
    if [ "$DRY" = 1 ]; then return 0; fi
    case $PLAN in
        repo|deb|rpm) pkg_probe; _now=${PKG_VER:-unknown} ;;
        *) _now=$INSTALLED_VERSION ;;
    esac
    if [ "$_now" = "$_ver" ] && [ "$PLAN" = repo ]; then say "Hexis $_now is already the newest version in the repository."
    elif [ "$_now" = "$_ver" ]; then say "Reinstalled Hexis $_now."
    else say "Updated Hexis $_ver -> $_now."; fi
}

do_update() {
    detect_installs
    if [ ! -s "$TMP/installs" ]; then
        die "Hexis is not installed by this installer, so there is nothing to update." "Install it with:  curl -fsS $SITE/install.sh | sh"
    fi
    _prefix_arg=$PREFIX
    while IFS='|' read -r _k _v _r _s <&3; do
        step "Updating: $(describe_install "$_k" "$_r") (now $_v)"
        PREFIX=$_prefix_arg
        update_one "$_k" "$_v" "$_r" "$_s"
    done 3<"$TMP/installs"
    say ""
    say "Done. Your vaults and settings were not touched by the update."
}

# Runs the plan that choose_plan, or update_one, decided on.
run_plan() {
    case $PLAN in
        apk) install_apk ;;
        repo) install_from_repo ;;
        deb) resolve_release; install_deb ;;
        rpm) resolve_release; install_rpm ;;
        tarball)
            resolve_release
            if [ "$MODE" = system ]; then install_tarball_deps; fi
            install_tarball ;;
        appimage) resolve_release; install_appimage ;;
    esac
}

# ------------------------------------------------------------------------------------------------ uninstall
# What Hexis keeps for one account is found by its own names only, never by a pattern: the data folder (vaults, settings,
# and what older builds' web view left there), the same names in the config, cache and state folders, and the launcher entry
# that the "app icon and name" setting writes (only when it carries Hexis's own marker line). Anything else is never looked at.
DATA_IDS="dev.habibullah.hexis app.passman.vault"

abs_or() { case $1 in /*) printf '%s' "$1" ;; *) printf '%s' "$2" ;; esac; }

collect_user_data() {
    USER_DATA=""
    if [ -z "${HOME:-}" ] || [ "$HOME" = / ]; then die "HOME is not set, so the folder Hexis keeps its data in cannot be found."; fi
    U_DATA=$(abs_or "${XDG_DATA_HOME:-}" "$HOME/.local/share")
    U_CONFIG=$(abs_or "${XDG_CONFIG_HOME:-}" "$HOME/.config")
    U_CACHE=$(abs_or "${XDG_CACHE_HOME:-}" "$HOME/.cache")
    U_STATE=$(abs_or "${XDG_STATE_HOME:-}" "$HOME/.local/state")
    _nl='
'
    for _base in "$U_DATA" "$U_CONFIG" "$U_CACHE" "$U_STATE"; do
        for _id in $DATA_IDS; do
            if [ -e "$_base/$_id" ] || [ -L "$_base/$_id" ]; then USER_DATA="$USER_DATA$_base/$_id$_nl"; fi
        done
    done
    _entry="$U_DATA/applications/dev.habibullah.hexis.desktop"
    if [ -f "$_entry" ] && grep -q '^X-Hexis-Managed=true' "$_entry" 2>/dev/null; then USER_DATA="$USER_DATA$_entry$_nl"; fi
}

data_label() {
    case $1 in
        */applications/*.desktop) printf '%s' "the launcher entry made by the app icon and name setting" ;;
        "$U_DATA"/*) printf '%s' "vaults and every setting Hexis keeps" ;;
        *) printf '%s' "cache or settings left by the screen" ;;
    esac
}

# Is the program running right now? A running Hexis would write its files again after they were removed.
refuse_if_running() {
    if have pgrep && pgrep -x -u "$(id -u)" hexis >/dev/null 2>&1; then
        die "Hexis is running. Close it first, so that nothing writes its files again after they are removed, and run this again."
    fi
}

announce_removal() {
    say "Uninstalling Hexis removes the program and EVERYTHING it keeps for the account whose home folder is $HOME."
    say "Nothing is kept: no vault, no setting, no cache, no cookie, no launcher entry."
    say ""
    while IFS= read -r _p; do
        [ -n "$_p" ] || continue
        say "  $_p"
        say "      $(data_label "$_p")"
    done <<EOF
$USER_DATA
EOF
    say ""
    say "Your vaults cannot be brought back from this computer afterwards. What is NOT removed: backups you made in S3 buckets"
    say "(they are not on this computer; delete them in your storage provider's console if you want them gone), and the data of"
    say "other accounts on this machine (each account removes its own the same way)."
    say "These files are deleted, not overwritten. The vault was sealed with your master password all along, but a deleted file can"
    say "stay on an SSD until the space is reused. For a stronger clean-up, first use Settings > Security > Remove Hexis from this"
    say "device inside the app: it destroys the keys, overwrites the files and then removes them."
    say "Use --keep-data to remove only the program."
    say ""
}

confirm_removal() {
    if [ "$ASSUME_YES" = 1 ] || [ "$DRY" = 1 ]; then return 0; fi
    _tty=${HEXIS_TTY:-/dev/tty}
    printf 'Type REMOVE and press Enter to remove all of it (anything else cancels): ' >&2
    _ans=""
    if ! { IFS= read -r _ans <"$_tty"; } 2>/dev/null; then
        printf '\n' >&2
        die "There is no terminal to ask on, so nothing was removed." \
            "Run it from a terminal, add --yes to confirm in advance, or add --keep-data to remove only the program."
    fi
    if [ "$_ans" != REMOVE ]; then die "Cancelled. Nothing was removed."; fi
}

remove_user_data() {
    while IFS= read -r _p; do
        [ -n "$_p" ] || continue
        case $_p in /?*) ;; *) die "refusing to remove a path that is not absolute: $_p" ;; esac
        case ${_p##*/} in
            dev.habibullah.hexis|app.passman.vault|dev.habibullah.hexis.desktop) ;;
            *) die "refusing to remove $_p: it is not one of Hexis's own names" ;;
        esac
        show rm -rf -- "$_p"
    done <<EOF
$USER_DATA
EOF
}

do_uninstall() {
    _found=0
    collect_user_data
    if [ "$KEEP_DATA" = 0 ] && [ -n "$USER_DATA" ]; then
        _found=1
        step "What will be removed"
        announce_removal
        confirm_removal
    fi
    if [ "$DRY" = 0 ]; then refuse_if_running; fi
    if have dpkg && dpkg_has "$DEB_PKG"; then
        _found=1; step "Removing the apt package"; run_root apt-get remove -y "$DEB_PKG"
    fi
    if have rpm && rpm -q "$RPM_PKG" >/dev/null 2>&1; then
        _found=1; step "Removing the rpm package"
        case $PM in zypper) run_root zypper --non-interactive remove "$RPM_PKG" ;; dnf|yum) run_root "$PM" remove -y "$RPM_PKG" ;; *) run_root rpm -e "$RPM_PKG" ;; esac
    fi
    if have pacman && pacman -Q "$DEB_PKG" >/dev/null 2>&1; then
        _found=1; step "Removing the pacman package"; run_root pacman -Rns --noconfirm "$DEB_PKG"
    fi
    _pc="$ROOTFS/etc/pacman.conf"
    if [ -f "$_pc" ] && grep -q '^# hexis-repo-begin' "$_pc"; then
        _found=1; step "Removing the pacman repository registration"
        _fpr=$(sed -n 's/^# hexis-repo-begin key=//p' "$_pc" | head -n 1)
        run_root sed -i '/^# hexis-repo-begin/,/^# hexis-repo-end/d' /etc/pacman.conf
        _sync=$(pacman-conf DBPath 2>/dev/null || echo /var/lib/pacman/)
        run_root rm -f "${_sync%/}/sync/hexis.db" "${_sync%/}/sync/hexis.db.sig"
        if [ -n "$_fpr" ]; then run_root pacman-key --delete "$_fpr" || true; fi
    fi
    for _f in /etc/apt/sources.list.d/hexis.sources /usr/share/keyrings/hexis-archive-keyring.gpg /etc/yum.repos.d/hexis.repo /etc/zypp/repos.d/hexis.repo; do
        if [ -e "$ROOTFS$_f" ]; then _found=1; step "Removing the repository registration $_f"; run_root rm -f "$_f"; fi
    done
    for _root in "${PREFIX:-}" "$HOME/.local" /usr/local; do
        [ -n "$_root" ] || continue
        _m="$_root/share/hexis/install-manifest"
        [ -f "$_m" ] || continue
        _found=1; step "Removing the files listed in $_m"
        _need=0; [ -w "$_root/share/hexis" ] || _need=1
        while IFS= read -r _line; do
            case $_line in
                file=*) if [ "$_need" = 1 ]; then run_root rm -f "$_root/${_line#file=}"; else show rm -f "$_root/${_line#file=}"; fi ;;
                dir=*) if [ "$_need" = 1 ]; then run_root rm -rf "$_root/${_line#dir=}"; else show rm -rf "$_root/${_line#dir=}"; fi ;;
            esac
        done <"$_m"
        if [ "$_need" = 1 ]; then run_root rmdir "$_root/share/hexis" 2>/dev/null || true; else rmdir "$_root/share/hexis" 2>/dev/null || true; fi
    done
    if [ "$KEEP_DATA" = 0 ] && [ -n "$USER_DATA" ]; then
        step "Removing everything Hexis kept for this account"
        remove_user_data
    fi
    if [ "$_found" = 0 ] && { [ "$KEEP_DATA" = 1 ] || [ -z "$USER_DATA" ]; }; then say "Nothing installed by this installer was found."; return 0; fi
    say ""
    if [ "$DRY" = 1 ]; then say "(dry run: nothing was removed)"; return 0; fi
    if [ "$KEEP_DATA" = 1 ]; then
        say "Hexis, the program, is removed. Your data was kept, as you asked:"
        while IFS= read -r _p; do
            if [ -n "$_p" ]; then say "  $_p"; fi
        done <<EOF
$USER_DATA
EOF
        say "Remove it too by running the uninstall again without --keep-data."
    else
        say "Hexis is removed, and so is everything it kept for this account: the vaults, the settings, the caches and the launcher entry."
        say "Nothing of Hexis is left in $HOME. Backups in S3 buckets and other accounts' data were not touched."
    fi
}

# ------------------------------------------------------------------------------------------------ choosing what to do
choose_plan() {
    PLAN=""; WHY=""
    if [ "$WANT_APK" = 1 ] || [ "$TERMUX" = 1 ]; then PLAN=apk; WHY="Android download"; return 0; fi

    # system-wide (package manager, /usr/local) or for this user only?
    if [ -z "$MODE" ]; then
        if [ "$IMMUTABLE" = 1 ] || [ "$PM" = none ]; then MODE=user
        elif [ "$IS_ROOT" = 1 ] || [ -n "$SUDO" ]; then MODE=system
        else MODE=user; fi
    fi
    if [ "$MODE" = system ] && [ "$IS_ROOT" = 0 ] && [ -z "$SUDO" ]; then
        die "A system-wide install needs root, and no sudo, doas, run0 or pkexec was found." "Use --user to install for your account only."
    fi

    case $METHOD in
        deb)
            [ "$PM" = apt ] || die "--method deb needs apt (this system: $PM)."
            [ "$MODE" = system ] || die "--method deb installs system-wide; drop --user."
            PLAN=deb; WHY="chosen with --method"; return 0 ;;
        rpm)
            case $PM in dnf|yum|zypper) ;; *) die "--method rpm needs dnf, yum or zypper (this system: $PM)." ;; esac
            [ "$MODE" = system ] || die "--method rpm installs system-wide; drop --user."
            PLAN=rpm; WHY="chosen with --method"; return 0 ;;
        tarball|appimage) PLAN=$METHOD; WHY="chosen with --method"; return 0 ;;
    esac

    # the signed package repository: stable channel, whole machine, a package manager that has one
    if [ "$METHOD" = repo ]; then
        [ "$MODE" = system ] || die "--method repo installs system-wide; drop --user."
        [ "$CHANNEL" = stable ] || die "The package repository carries stable releases only. Use --method deb|rpm|tarball|appimage."
    fi
    if [ "$METHOD" = repo ] || { [ "$METHOD" = auto ] && [ "$MODE" = system ] && [ "$CHANNEL" = stable ]; }; then
        case $PM in
            apt|dnf|yum|zypper|pacman)
                if repo_available; then PLAN=repo; WHY="the signed Hexis package repository; later updates arrive through $PM"; return 0; fi
                if [ "$METHOD" = repo ]; then
                    die "The package repository at $SITE is not published yet." "It appears with the first stable release. Use --nightly, or --method deb|rpm|tarball|appimage."
                fi
                warn "The package repository is not published yet (it appears with the first stable release); using the release files instead."
                ;;
            *)
                if [ "$METHOD" = repo ]; then die "There is no package repository for this system ($PM). Use --method tarball or appimage."; fi
                ;;
        esac
    fi

    if [ "$MODE" = system ]; then
        case $PM in
            apt) PLAN=deb; WHY="Debian family: the .deb, installed with apt" ;;
            dnf|yum|zypper) PLAN=rpm; WHY="rpm family: the .rpm, installed with $PM" ;;
            pacman) PLAN=tarball; WHY="Arch family: the tarball into /usr/local, libraries from pacman (no package repository yet)" ;;
            *) MODE=user ;;
        esac
    fi
    if [ -z "$PLAN" ]; then
        if [ "$HOST_WEBKIT" = 1 ]; then PLAN=tarball; WHY="user install; WebKitGTK 4.1 and GTK 3 are already here, so the small tarball is enough"
        else PLAN=appimage; WHY="user install; the AppImage carries its own libraries"; fi
    fi
}

main() {
    parse_args "$@"
    detect_platform
    if [ "$DETECT_ONLY" = 1 ]; then print_detect; exit 0; fi
    check_supported
    if [ "$UNINSTALL" = 1 ]; then do_uninstall; exit 0; fi

    TMP=$(mktemp -d "${TMPDIR:-/tmp}/hexis-install.XXXXXX") || die "could not create a temporary folder"
    trap 'rm -rf "$TMP"' EXIT
    trap 'exit 130' INT HUP TERM
    umask 022
    chmod 755 "$TMP"
    INSTALLED_VERSION=""; INSTALLED_SHA=""; S=""

    if [ "$CHECK" = 1 ] || [ "$UPDATE" = 1 ]; then
        if [ "$WANT_APK" = 1 ] || [ "$TERMUX" = 1 ]; then
            die "Android has no installer to check or update: download the newest APK with --apk and install it over the old one." \
                "(Android accepts it only when it was signed with the same key; the file name says -testkey when it was not.)"
        fi
        say "Hexis installer ($DISTRO_NAME, $MACHINE, $LIBC${GLIBC_VER:+ $GLIBC_VER})"
        if [ "$CHECK" = 1 ]; then do_check; else
            if [ "$DRY" = 1 ]; then say "(dry run: nothing will be changed)"; fi
            do_update
        fi
        exit 0
    fi

    say "Hexis installer ($DISTRO_NAME, $MACHINE, $LIBC${GLIBC_VER:+ $GLIBC_VER})"
    choose_plan
    say "Plan: $PLAN ($WHY)"
    if [ "$DRY" = 1 ]; then say "(dry run: nothing will be changed)"; fi
    if [ "$PLAN" != apk ]; then set_paths; fi
    run_plan

    if [ "$DRY" = 1 ] || [ "$PLAN" = apk ]; then exit 0; fi
    say ""
    say "Hexis is installed. Start it from your application menu, or run: hexis"
    case $PLAN in
        tarball|appimage)
            case ":$PATH:" in *":$P_BIN:"*) ;; *) say "Note: $P_BIN is not in your PATH; the application-menu entry works anyway." ;; esac
            say "Update it later with: hexis-update   (or: curl -fsS $SITE/install.sh | sh -s -- --update)" ;;
        repo) say "Update it later with your package manager ($PM), or: curl -fsS $SITE/install.sh | sh -s -- --update" ;;
        *) say "Update it later with: curl -fsS $SITE/install.sh | sh -s -- --update" ;;
    esac
    say "Remove it with:       curl -fsS $SITE/install.sh | sh -s -- --uninstall   (lists everything it removes, asks first; nothing is kept)"
    say "Pre-1.0 and not independently audited: keep a separate backup of anything you cannot lose."
}

main "$@"
