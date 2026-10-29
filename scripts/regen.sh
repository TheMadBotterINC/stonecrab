#!/usr/bin/env bash
# Usage: scripts/regen.sh | --self-test | --help
#
# The regen gate of ADR-0019 decision 9 (slice S67): generated code is exactly what the
# generator makes from the vendored mspec files, deterministically. In order:
#
#   1. The one network step: `cargo fetch --locked` for the generator workspace
#      (tools/stonecrab-codegen) and `cargo deny fetch db index` (the RustSec advisory
#      database and the crates index the `yanked = "deny"` policy needs). Everything after
#      it runs with --frozen (--locked and --offline), so a network attempt fails.
#   2. The generator workspace: tools/stonecrab-codegen/deny.toml against the root deny.toml
#      (only comment lines may differ), `cargo fmt --check`, `cargo clippy --all-targets
#      -- -D warnings`, `cargo test` and `cargo deny check`.
#   3. Each vendored mspec against the sha256 that upstream/manifest.toml records (the
#      generator checks it again and refuses to run on a mismatch).
#   4. For every protocol under tools/stonecrab-codegen/protocols/: the generator twice into
#      two scratch directories, which must be byte-identical; then in place. The tree must
#      not change: a copy of the protocol's generated.rs and generated/ taken before must
#      equal them after (diff -r, which an edit, a new file and a deleted file all fail),
#      and `git status --porcelain --untracked-files=no` must be empty for them. On a
#      clean checkout, as in CI, that is decision 9's `git status --porcelain`; the copy
#      also works before the generated files are committed.
#
#   --self-test   check that the comparison of step 4 fails on an edited, an added and a
#                 removed generated file (a scratch copy; the tree is not touched)
#
# Needs cargo, rustfmt and cargo-deny on PATH (mise provides the pinned ones) and git.
# Exit status: 0 clean, 1 a check failed, 2 usage error.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
tool="tools/stonecrab-codegen"
cd "$root"

usage() {
  sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//'
}

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

# The generator, built offline from its own workspace.
codegen() {
  cargo run --frozen --quiet --manifest-path "$tool/Cargo.toml" -- "$@"
}

# Fails (exit 1) when two generated trees differ in any file, added or removed ones included.
same_tree() { # first second
  if ! diff -r "$1" "$2" > /dev/null; then
    echo "generated output differs between $1 and $2:" >&2
    diff -r "$1" "$2" >&2 || true
    return 1
  fi
}

sha256() { # file
  if command -v sha256sum > /dev/null; then
    sha256sum "$1" | cut -d ' ' -f 1
  else
    shasum -a 256 "$1" | cut -d ' ' -f 1
  fi
}

# The `output` of a protocol options file.
output_of() { # options
  sed -n 's/^output = "\(.*\)"$/\1/p' "$1"
}

self_test() {
  local failures=0 options first
  options="$(ls "$tool"/protocols/*.toml | head -n 1)"
  codegen generate "$options" "$scratch/base"
  first="$(ls "$scratch/base/generated"/*.rs | head -n 1)"
  for case in edited added removed; do
    rm -rf "$scratch/copy"
    cp -R "$scratch/base" "$scratch/copy"
    local target="$scratch/copy/generated/$(basename "$first")"
    case "$case" in
      edited) printf '// a hand edit\n' >> "$target" ;;
      added) printf '// not generated\n' > "$scratch/copy/generated/extra.rs" ;;
      removed) rm "$target" ;;
    esac
    if same_tree "$scratch/base" "$scratch/copy" 2> /dev/null; then
      echo "self-test FAIL $case: the comparison passed" >&2
      failures=$((failures + 1))
    else
      echo "self-test ok   $case: the comparison fails"
    fi
  done
  [ "$failures" -eq 0 ]
}

case "${1:-}" in
  -h | --help)
    usage
    exit 0
    ;;
  --self-test)
    cargo fetch --locked --manifest-path "$tool/Cargo.toml"
    self_test
    exit $?
    ;;
  '') ;;
  *)
    echo "usage: scripts/regen.sh | --self-test | --help" >&2
    exit 2
    ;;
esac

echo "== 1. fetch (the only network step)"
cargo fetch --locked --manifest-path "$tool/Cargo.toml"
(cd "$tool" && cargo deny --locked fetch db index)

echo "== 2. the generator workspace, offline"
if ! diff <(grep -v '^#' "$tool/deny.toml") <(grep -v '^#' deny.toml) > /dev/null; then
  echo "$tool/deny.toml has drifted from deny.toml (only comment lines may differ)" >&2
  exit 1
fi
(
  cd "$tool"
  cargo fmt --check
  cargo clippy --frozen --all-targets -- -D warnings
  cargo test --frozen
  cargo deny --frozen check
)

echo "== 3. vendored mspec hashes"
# Pairs vendored_path and sha256 of each [[mspec]] entry, in file order.
pairs="$(awk -F' = ' '
  /^\[\[mspec\]\]/ { in_mspec = 1; next }
  /^\[/ { in_mspec = 0 }
  in_mspec && $1 == "vendored_path" { gsub(/"/, "", $2); path = $2 }
  in_mspec && $1 == "sha256" { gsub(/"/, "", $2); print path " " $2 }
' upstream/manifest.toml)"
if [ -z "$pairs" ]; then
  echo "upstream/manifest.toml lists no [[mspec]] entry" >&2
  exit 1
fi
while read -r path expected; do
  actual="$(sha256 "$path")"
  if [ "$actual" != "$expected" ]; then
    echo "$path: sha256 $actual, upstream/manifest.toml records $expected" >&2
    exit 1
  fi
  echo "ok   $path"
done <<< "$pairs"

echo "== 4. determinism and a clean tree"
for options in "$tool"/protocols/*.toml; do
  name="$(basename "$options" .toml)"
  codegen generate "$options" "$scratch/$name-1"
  codegen generate "$options" "$scratch/$name-2"
  same_tree "$scratch/$name-1" "$scratch/$name-2"
  out="$(output_of "$options")"
  mkdir -p "$scratch/$name-before/generated"
  if [ -e "$out/generated.rs" ]; then
    cp "$out/generated.rs" "$scratch/$name-before/"
  fi
  if [ -d "$out/generated" ]; then
    cp -R "$out/generated/." "$scratch/$name-before/generated/"
  fi
  codegen generate "$options"
  mkdir -p "$scratch/$name-after"
  cp "$out/generated.rs" "$scratch/$name-after/"
  cp -R "$out/generated" "$scratch/$name-after/"
  if ! same_tree "$scratch/$name-before" "$scratch/$name-after"; then
    echo "the generated code of $name in the tree is not what the generator makes" >&2
    exit 1
  fi
  dirty="$(git status --porcelain --untracked-files=no -- "$out/generated.rs" "$out/generated")"
  if [ -n "$dirty" ]; then
    echo "the committed generated code of $name is not what the generator makes:" >&2
    echo "$dirty" >&2
    exit 1
  fi
  echo "ok   $name: two runs identical, the tree is unchanged"
done
