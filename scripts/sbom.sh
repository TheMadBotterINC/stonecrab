#!/usr/bin/env bash
# Usage: scripts/sbom.sh [--help]
#
# Release evidence (ADR-0017): the CycloneDX SBOM of the one published crate and the file
# list of its release archive, written to target/release-evidence/:
#
#   sbom/stonecrab.cdx.json  cargo cyclonedx --format json (cargo-cyclonedx 0.5.9)
#   package-list.txt         cargo package --locked -p stonecrab --list, to review against
#                            packaging/stonecrab.txt
#
# `mise run sbom` and the release-evidence workflow (.github/workflows/release-evidence.yml)
# both run this script, so the two cannot drift. cargo-cyclonedx has no output directory
# option and writes <crate>.cdx.json next to each Cargo.toml, so the script moves the file
# under target/release-evidence/sbom/; the source tree is left as it was found.
# SOURCE_DATE_EPOCH is the commit time so that the same commit gives the same timestamps.
#
# Needs cargo-cyclonedx 0.5.9 on PATH (pinned in mise.toml). Exit status: 0 success,
# non-zero when a command fails or no SBOM was produced.
set -euo pipefail

case "${1:-}" in
  -h | --help)
    sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  '') ;;
  *)
    echo "usage: scripts/sbom.sh" >&2
    exit 2
    ;;
esac

cd "$(dirname "$0")/.."
out=target/release-evidence
rm -rf "$out"
mkdir -p "$out/sbom"

# cargo package refuses a tree with uncommitted changes in a crate. Release evidence is
# made from a clean checkout; SBOM_ALLOW_DIRTY=1 lets a developer try the script on a
# working tree.
allow_dirty=""
if [ "${SBOM_ALLOW_DIRTY:-}" = 1 ]; then
  allow_dirty=1
fi

SOURCE_DATE_EPOCH="$(git log -1 --format=%ct)"
export SOURCE_DATE_EPOCH

cargo cyclonedx --format json --all-features --spec-version 1.5

count=0
for file in crates/*/*.cdx.json; do
  [ -f "$file" ] || continue
  mv "$file" "$out/sbom/"
  count=$((count + 1))
done
# cargo-cyclonedx writes the absolute path of the checkout into bom-ref and purl strings
# (path+file:///...). Replace it so a published SBOM does not carry a username or a runner
# layout, then fail if any absolute home or temp path remains.
root="$(pwd -P)"
for file in "$out"/sbom/*.json; do
  sed "s#file://$root#file://.#g; s#$root#.#g" "$file" > "$file.tmp"
  mv "$file.tmp" "$file"
done
if grep -Eq '/(Users|home|private|var|tmp)/' "$out"/sbom/*.json; then
  echo "sbom: an absolute path remains in an SBOM" >&2
  exit 1
fi
if [ "$count" -eq 0 ]; then
  echo "sbom: cargo cyclonedx produced no file under crates/" >&2
  exit 1
fi

cargo package --locked -p stonecrab --list ${allow_dirty:+--allow-dirty} \
  > "$out/package-list.txt"
echo "sbom: $count SBOM file, package-list.txt in $out"
ls "$out/sbom"
