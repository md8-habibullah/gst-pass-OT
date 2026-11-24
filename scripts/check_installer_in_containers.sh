#!/usr/bin/env bash
# Run the real installer (this working tree's scripts/install.sh and scripts/tests/container_case.sh) in distribution containers,
# against the packages the newest green CI run built. Run it whenever you change either file, BEFORE you push: Debian and Ubuntu
# run `sh` as dash, Fedora, Arch and openSUSE as bash, and only a run in the containers shows the difference.
#   scripts/check_installer_in_containers.sh                    debian:12 ubuntu:22.04 fedora:latest
#   scripts/check_installer_in_containers.sh archlinux:latest   or the images you name
# Needs docker and a logged-in gh. The images run one after another (the machine this was written on has little RAM).
set -euo pipefail
cd "$(dirname "$0")/.."
command -v docker >/dev/null || { echo "docker is needed" >&2; exit 2; }
command -v gh >/dev/null || { echo "gh (logged in) is needed to fetch the packages CI built" >&2; exit 2; }
if [ $# -eq 0 ]; then set -- debian:12 ubuntu:22.04 fedora:latest; fi
run_id=$(gh run list --workflow CI --branch main --status success --limit 1 --json databaseId --jq '.[0].databaseId')
if [ -z "$run_id" ] || [ "$run_id" = null ]; then echo "no green CI run on main to take packages from" >&2; exit 1; fi
# a new, empty folder: downloaded files are data, never code
dest=$(mktemp -d)
trap 'rm -rf "$dest"' EXIT
echo "== packages from CI run $run_id"
gh run download "$run_id" -n linux-bundles -D "$dest"
bash scripts/test_installer_containers.sh "$dest" "$@"
