#!/usr/bin/env bash
# Sets up the Android build toolchain for Hexis at USER level (no sudo, nothing outside your home directory):
#   ~/Android/Sdk          Android SDK (command-line tools, platform 36, build-tools 36.0.0, NDK r28c)
#   ~/.local/jdks/temurin-21   JDK 21 (Gradle and the Android Gradle plugin do not run on newer JDKs)
#   rustup target aarch64-linux-android (and the other three Android targets with --all-targets)
#
#   scripts/setup_android_toolchain.sh phase1   download + verify the command-line tools and the JDK, add the Rust target(s)
#   scripts/setup_android_toolchain.sh phase2   install platform / build-tools / NDK with sdkmanager (needs the licences accepted)
#   scripts/setup_android_toolchain.sh env      print the environment variables to use (eval "$(... env)")
#
# The Android SDK licences are an agreement between YOU and Google. This script never accepts them for you:
# after phase1, run   yes | ~/Android/Sdk/cmdline-tools/latest/bin/sdkmanager --licenses   yourself (JAVA_HOME below).
# Every download is checked against the checksum the vendor publishes (Google: repository2-3.xml, SHA-1; Adoptium: SHA-256).
set -euo pipefail

SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
JDKS="$HOME/.local/jdks"
JDK_HOME_DIR="$JDKS/temurin-21"
CACHE="$HOME/.cache/hexis-android"
REPO="https://dl.google.com/android/repository"

# Pinned from https://dl.google.com/android/repository/repository2-3.xml and the Adoptium API on 2026-11-24.
CMDLINE_ZIP="commandlinetools-linux-16111833_latest.zip"; CMDLINE_SHA1="e025545c62a8e64c7559119566a569fb1dec5f60"
JDK_URL="https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.13.1%2B2/OpenJDK21U-jdk_x64_linux_hotspot_21.0.13.1_2.tar.gz"
JDK_SHA256="bb8a93f9ae6e7a693b2b721e85dbeff26ae1a77447985a8d7afcdd9c7cf6222e"
PACKAGES=("platform-tools" "platforms;android-36" "build-tools;36.0.0" "ndk;28.2.13676358")

log() { printf '[android-setup] %s\n' "$*"; }
fetch() { # fetch <url> <file>   (resumable, no progress noise)
  mkdir -p "$CACHE"; curl -fL --retry 3 --retry-delay 3 -C - --no-progress-meter -A "hexis-build-check" -o "$2" "$1"
}

phase1() {
  mkdir -p "$CACHE" "$SDK/cmdline-tools" "$JDKS"
  if [ ! -x "$SDK/cmdline-tools/latest/bin/sdkmanager" ]; then
    log "downloading command-line tools ($CMDLINE_ZIP)"
    fetch "$REPO/$CMDLINE_ZIP" "$CACHE/$CMDLINE_ZIP"
    echo "$CMDLINE_SHA1  $CACHE/$CMDLINE_ZIP" | sha1sum -c - >&2
    rm -rf "$CACHE/cmdline-extract"; mkdir -p "$CACHE/cmdline-extract"
    unzip -q "$CACHE/$CMDLINE_ZIP" -d "$CACHE/cmdline-extract"
    rm -rf "$SDK/cmdline-tools/latest"; mv "$CACHE/cmdline-extract/cmdline-tools" "$SDK/cmdline-tools/latest"
    log "command-line tools installed"
  else log "command-line tools already present"; fi

  if [ ! -x "$JDK_HOME_DIR/bin/java" ]; then
    log "downloading Temurin JDK 21"
    fetch "$JDK_URL" "$CACHE/temurin21.tar.gz"
    echo "$JDK_SHA256  $CACHE/temurin21.tar.gz" | sha256sum -c - >&2
    rm -rf "$CACHE/jdk-extract"; mkdir -p "$CACHE/jdk-extract"
    tar -xzf "$CACHE/temurin21.tar.gz" -C "$CACHE/jdk-extract"
    rm -rf "$JDK_HOME_DIR"; mv "$CACHE"/jdk-extract/jdk-21* "$JDK_HOME_DIR"
    log "JDK installed at $JDK_HOME_DIR"
  else log "JDK already present"; fi

  log "adding Rust Android target(s)"
  if [ "${2:-}" = "--all-targets" ]; then rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
  else rustup target add aarch64-linux-android; fi

  log "phase 1 done. Next, YOU accept the Android SDK licences (they are an agreement with Google):"
  echo "  JAVA_HOME=$JDK_HOME_DIR yes | $SDK/cmdline-tools/latest/bin/sdkmanager --sdk_root=$SDK --licenses"
  echo "then run:  scripts/setup_android_toolchain.sh phase2"
}

phase2() {
  [ -x "$SDK/cmdline-tools/latest/bin/sdkmanager" ] || { echo "run phase1 first" >&2; exit 1; }
  export JAVA_HOME="$JDK_HOME_DIR"
  if ! ls "$SDK/licenses"/* >/dev/null 2>&1; then echo "No accepted licences found in $SDK/licenses. Accept them first (see the end of phase1)." >&2; exit 1; fi
  log "installing: ${PACKAGES[*]}"
  "$SDK/cmdline-tools/latest/bin/sdkmanager" --sdk_root="$SDK" "${PACKAGES[@]}"
  log "phase 2 done"
  env_cmd
}

env_cmd() {
  cat <<EOF
export JAVA_HOME="$JDK_HOME_DIR"
export ANDROID_HOME="$SDK"
export ANDROID_SDK_ROOT="$SDK"
export NDK_HOME="$SDK/ndk/28.2.13676358"
export PATH="\$JAVA_HOME/bin:$SDK/platform-tools:\$PATH"
EOF
}

case "${1:-}" in
  phase1) phase1 "$@" ;;
  phase2) phase2 ;;
  env) env_cmd ;;
  *) sed -n '2,13p' "$0"; exit 2 ;;
esac
