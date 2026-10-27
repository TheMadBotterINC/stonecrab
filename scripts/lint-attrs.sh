#!/usr/bin/env bash
# Usage: scripts/lint-attrs.sh [--root DIR] | --self-test | --help
#
# Enforces CODING_STANDARDS section 5.1: crate-level lint attributes in lib.rs are
# prohibited except `#![forbid(unsafe_code)]`, `#![cfg_attr(docsrs, ...)]` and the
# per-crate promotions (`#![deny(<lint>)]`) in the PERMITTED table below. This script
# replaces the `xtask lint-attrs` named in the standards (ADR-0005).
#
# The check is line based: an attribute must sit on one line with nothing after it (no
# multi-line `#![deny(\n lint\n)]`, no trailing comment), or it is reported.
#
# The per-module promotions (clippy::indexing_slicing, clippy::arithmetic_side_effects,
# clippy::as_conversions) are outer `#[deny(...)]` attributes on the `pub mod` declarations of
# lib.rs (they may span several lines). Each is checked against the MODULE_PERMITTED table; any
# other outer allow/warn/expect/forbid attribute in lib.rs is reported.
#
# Scans <root>/crates/*/src/lib.rs. Stonecrab is one crate, so there is one real lib.rs. The
# module root files (src/{codec,transport,modbus,sim,sensors,testkit}.rs) must carry no inner
# lint-level attribute at all; that is checked too.
# A crate directory that is absent is simply not scanned; a crate that is present but missing
# from the table fails, so a new crate must be added here in the PR that creates it.
#
#   --root DIR    scan DIR instead of the repository root (used by --self-test)
#   --self-test   run the fixture trees under scripts/testdata/lint-attrs/ and check
#                 that the clean tree passes and every bad tree fails
#
# Exit status: 0 clean, 1 violations found, 2 usage error.
# Portable bash 3.2+ and POSIX grep/sed/awk: runs on macOS, Linux and Git Bash.
set -euo pipefail

# crate directory name | space-separated lints that crate may promote to deny.
# CODING_STANDARDS section 5.1; the one crate promotes missing_docs and as_conversions.
PERMITTED="
stonecrab|missing_docs clippy::as_conversions
"

# module name | space-separated lints that `pub mod <name>` may promote with an outer #[deny].
MODULE_PERMITTED="
codec|clippy::indexing_slicing clippy::arithmetic_side_effects clippy::as_conversions
modbus|clippy::indexing_slicing clippy::arithmetic_side_effects clippy::as_conversions
sim|clippy::indexing_slicing clippy::arithmetic_side_effects clippy::as_conversions
transport|clippy::indexing_slicing clippy::as_conversions
sensors|
testkit|
"

MODULE_ROOTS="codec transport modbus sim sensors testkit"

LEVEL_WORDS='(allow|warn|deny|forbid|expect)[[:space:]]*\('

script_dir=$(cd "$(dirname "$0")" && pwd)
repo_root=$(cd "$script_dir/.." && pwd)

usage() {
  sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'
}

# Prints the permitted lints for a crate; returns 1 when the crate is not in the table.
permitted_for() {
  printf '%s\n' "$PERMITTED" | awk -F'|' -v n="$1" '
    $1 == n { found = 1; print $2 }
    END { exit !found }'
}

trim() {
  printf '%s' "$1" | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//'
}

in_list() { # word list
  case " $2 " in *" $1 "*) return 0 ;; esac
  return 1
}

violations=0

report() { # file line text reason
  printf 'lint-attrs: %s:%s: %s\n    %s\n' "$1" "$2" "$3" "$4" >&2
  violations=$((violations + 1))
}

check_crate() { # root-relative path to lib.rs, crate name
  local rel=$1 name=$2 file permitted entry lineno text body lint
  file="$ROOT/$rel"
  if ! permitted=$(permitted_for "$name"); then
    report "$rel" 1 "crate '$name' has no row in the PERMITTED table" \
      "add it to scripts/lint-attrs.sh in the PR that creates the crate"
    return
  fi
  while IFS= read -r entry; do
    [ -n "$entry" ] || continue
    lineno=${entry%%:*}
    text=$(trim "${entry#*:}")
    if [ "$text" = '#![forbid(unsafe_code)]' ]; then
      continue
    fi
    if printf '%s\n' "$text" | grep -Eq '^#!\[deny\([^()]*\)\]$'; then
      body=${text#'#![deny('}
      body=${body%')]'}
      local ok=1
      # Lint names never contain spaces, so commas become the word separator.
      for lint in $(printf '%s' "$body" | tr ',' ' '); do
        if ! in_list "$lint" "$permitted"; then
          ok=0
        fi
      done
      if [ -z "$(trim "$body")" ]; then
        ok=0
      fi
      if [ "$ok" -eq 1 ]; then
        continue
      fi
    fi
    report "$rel" "$lineno" "$text" \
      "not permitted for $name (permitted promotions: ${permitted:-none}); section 5.1"
  done < <(grep -nE "^[[:space:]]*#!\\[.*${LEVEL_WORDS}" "$file" || true)
}

# Prints "lineno|module|attribute" for every outer attribute in a file, with multi-line
# attributes joined; module is the name on the declaration that follows, or "-".
outer_attrs() { # file
  awk '
    function flush(name,   i) {
      for (i = 1; i <= n; i++) print ln[i] "|" name "|" at[i]
      n = 0
    }
    /^[[:space:]]*#\[/ && !open {
      open = 1; cur = ""; start = NR; depth = 0
    }
    open {
      t = $0; gsub(/[[:space:]]+/, "", t); cur = cur t
      o = gsub(/\[/, "[", t); c = gsub(/\]/, "]", t)
      depth += o - c
      if (depth <= 0) { n++; ln[n] = start; at[n] = cur; open = 0 }
      next
    }
    /^[[:space:]]*(\/\/.*)?$/ { next }
    {
      name = "-"
      if (match($0, /^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?mod[[:space:]]+[A-Za-z0-9_]+/)) {
        name = substr($0, RSTART, RLENGTH); sub(/.*mod[[:space:]]+/, "", name)
      }
      flush(name)
    }
    END { flush("-") }' "$1"
}

check_outer() { # root-relative path to lib.rs
  local rel=$1 entry lineno rest module attr body lint permitted ok
  while IFS= read -r entry; do
    [ -n "$entry" ] || continue
    lineno=${entry%%|*}; rest=${entry#*|}
    module=${rest%%|*}; attr=${rest#*|}
    printf '%s\n' "$attr" | grep -Eq "(^#\\[|[ ,(])${LEVEL_WORDS}" || continue
    permitted=$(printf '%s\n' "$MODULE_PERMITTED" | awk -F'|' -v n="$module" '$1 == n { print $2 }')
    if printf '%s\n' "$attr" | grep -Eq '^#\[deny\([^()]*\)\]$'; then
      body=${attr#'#[deny('}
      body=${body%')]'}
      ok=1
      for lint in $(printf '%s' "$body" | tr ',' ' '); do
        in_list "$lint" "$permitted" || ok=0
      done
      [ -n "$body" ] || ok=0
      [ "$ok" -eq 1 ] && continue
    fi
    report "$rel" "$lineno" "$attr" \
      "outer lint attribute on '$module' not permitted (module promotions: ${permitted:-none}); section 5.1"
  done < <(outer_attrs "$ROOT/$rel")
}

check_module_root() { # root-relative path
  local rel=$1 entry
  while IFS= read -r entry; do
    [ -n "$entry" ] || continue
    report "$rel" "${entry%%:*}" "$(trim "${entry#*:}")" \
      "module root files carry no inner lint attributes; section 5.1"
  done < <(grep -nE "^[[:space:]]*#!\\[.*${LEVEL_WORDS}" "$ROOT/$rel" || true)
}

scan() { # root
  ROOT=$1
  violations=0
  local lib dir name rel m scanned=0
  for lib in "$ROOT"/crates/*/src/lib.rs; do
    [ -f "$lib" ] || continue
    rel=${lib#"$ROOT"/}
    dir=${rel%/src/lib.rs}
    name=${dir#*/}
    check_crate "$rel" "$name"
    check_outer "$rel"
    for m in $MODULE_ROOTS; do
      [ -f "$ROOT/$dir/src/$m.rs" ] && check_module_root "$dir/src/$m.rs"
    done
    scanned=$((scanned + 1))
  done
  if [ "$scanned" -eq 0 ]; then
    printf 'lint-attrs: no crates/*/src/lib.rs under %s\n' "$ROOT" >&2
    return 1
  fi
  if [ "$violations" -gt 0 ]; then
    printf 'lint-attrs: FAIL, %s violation(s) in %s lib.rs file(s)\n' "$violations" "$scanned" >&2
    return 1
  fi
  printf 'lint-attrs: ok, %s lib.rs file(s) checked\n' "$scanned"
}

self_test() {
  local base="$script_dir/testdata/lint-attrs" failures=0 dir name rc
  for dir in "$base"/pass-* "$base"/fail-*; do
    [ -d "$dir" ] || continue
    name=$(basename "$dir")
    rc=0
    "$0" --root "$dir" >/dev/null 2>&1 || rc=$?
    case "$name" in
      pass-*) [ "$rc" -eq 0 ] && printf 'self-test ok   %s (exit 0)\n' "$name" \
        || { printf 'self-test FAIL %s: expected exit 0, got %s\n' "$name" "$rc"; failures=$((failures + 1)); } ;;
      fail-*) [ "$rc" -eq 1 ] && printf 'self-test ok   %s (exit 1)\n' "$name" \
        || { printf 'self-test FAIL %s: expected exit 1, got %s\n' "$name" "$rc"; failures=$((failures + 1)); } ;;
    esac
  done
  [ "$failures" -eq 0 ]
}

main() {
  case "${1:-}" in
    -h | --help) usage ;;
    --self-test) self_test ;;
    --root)
      [ -n "${2:-}" ] && [ -d "$2" ] || { echo "lint-attrs: --root needs a directory" >&2; exit 2; }
      scan "$(cd "$2" && pwd)"
      ;;
    '') scan "$repo_root" ;;
    *) usage >&2; exit 2 ;;
  esac
}

main "$@"
