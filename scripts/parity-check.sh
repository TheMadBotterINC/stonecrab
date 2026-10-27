#!/usr/bin/env bash
# Usage: scripts/parity-check.sh [--root DIR] | --self-test | --help
#
# Keeps the parity records (docs/parity/*.md) and the Rust sources honest about each
# other (docs/ALPHA_PLAN.md S31, ADR-0005 addendum). Two greps, both blocking:
#
#   (a) every `docs/parity/<file>.md#<ROW-ID>` reference in crates/*/src/**/*.rs (the
#       `# Compatibility: docs/parity/<file>.md#<ROW-ID>` rustdoc sections of CODING_STANDARDS
#       12.2 and any prose cross-reference of the same shape) names a file that exists
#       and a row id that appears in that file;
#   (b) every row whose status starts with `implemented` or `automated-verified` is named
#       at least once, as the bare row id, in crates/*/src/**/*.rs (which includes the
#       testkit module) or crates/*/tests/**/*.rs (a `# Compatibility` section, a
#       `// covers:` comment, a test name or an assertion message).
#
# Because the exit criterion of S31 is "no implemented row without a test id", a third
# check rides along: such a row must also carry at least one backticked test id in its
# test-id column.
#
# How a row id appears in the records: the first cell of a table row, an upper-case id of
# two or more hyphen-joined groups (`MB-TCP-CONNECT`, `CORE-URI-GRAMMAR`). That is the
# only form the five files use; headings are not ids. A divergence row reuses the id of
# the behavior row it qualifies, so an id may appear in two tables of one file; only the
# rows of a table with a `Status` column are subject to (b). The status column is the one
# headed `Status`, the test-id column the one headed with `Test ids` (any prefix such as
# `Planned`), `Regression test id` or `Stonecrab handling and test id`.
#
#   --root DIR    check DIR instead of the repository root (used by --self-test)
#   --self-test   run the fixture trees under scripts/testdata/parity-check/ and check
#                 that the clean trees pass and every bad tree fails
#
# Every failure prints its file, line and id on stderr. Exit status: 0 clean, 1 a finding
# (or a malformed table), 2 usage error. Portable bash 3.2+ and POSIX grep/sed/awk/find:
# runs on macOS, Linux and Git Bash.
set -euo pipefail

script_dir=$(cd "$(dirname "$0")" && pwd)
repo_root=$(cd "$script_dir/.." && pwd)

usage() {
  sed -n '2,34p' "$0" | sed 's/^# \{0,1\}//'
}

# Upper-case, hyphen-joined id as an extended regular expression (no anchors).
ID_RE='[A-Z][A-Z0-9]*(-[A-Z0-9]+)+'

# Prints one line per row id, fields separated by the ASCII unit separator (a status may
# be empty, which a tab separator would collapse), in docs/parity/*.md:
#   file  line  id  status  test-cell-has-backticked-id(0|1)  has-status-column(0|1)
# and, on stderr via a leading "ERR" line on stdout, any malformed table.
extract_rows() { # parity-directory
  local f
  for f in "$1"/*.md; do
    [ -f "$f" ] || continue
    awk -v file="docs/parity/$(basename "$f")" -v idre="^${ID_RE}\$" '
      function trim(s) { sub(/^[ \t]+/, "", s); sub(/[ \t]+$/, "", s); return s }
      FNR == 1 { intable = 0 }
      $0 !~ /^\|/ { intable = 0; next }
      {
        line = $0
        gsub(/\\\|/, "\001", line)          # an escaped pipe is text, not a separator
        sub(/^\|/, "", line)
        sub(/\|[ \t]*$/, "", line)
        n = split(line, cell, "|")
        for (i = 1; i <= n; i++) cell[i] = trim(cell[i])
        if (!intable) {                      # the header row of a new table
          intable = 1; hn = n; statuscol = 0; testcol = 0
          for (i = 1; i <= n; i++) {
            h = tolower(cell[i])
            if (h == "status") statuscol = i
            if (h ~ /test id/ || h ~ /regression test/) testcol = i
          }
          next
        }
        sep = 1
        for (i = 1; i <= n; i++) if (cell[i] !~ /^:?-+:?$/) sep = 0
        if (sep) next                        # the |---|---| row
        if (cell[1] !~ idre) next            # a key table or prose table, not a row id
        if (n != hn) {
          printf "ERR\037%s\037%d\037%s\037a row has %d cells but its table header has %d\n", file, FNR, cell[1], n, hn
          next
        }
        status = statuscol ? cell[statuscol] : ""
        hastest = 0
        if (testcol && cell[testcol] ~ /`[^`]+`/) hastest = 1
        printf "ROW\037%s\037%d\037%s\037%s\037%d\037%d\n", file, FNR, cell[1], status, hastest, (statuscol ? 1 : 0)
      }
    ' "$f"
  done
}

violations=0

report() { # file line id reason
  printf 'parity-check: %s:%s: %s: %s\n' "$1" "$2" "$3" "$4" >&2
  violations=$((violations + 1))
}

scan() { # root
  local root=$1 rows tokens_file
  cd "$root"
  if [ ! -d docs/parity ]; then
    printf 'parity-check: no docs/parity directory under %s\n' "$root" >&2
    return 1
  fi
  violations=0
  rows=$(extract_rows docs/parity)
  if [ -z "$rows" ]; then
    printf 'parity-check: no row ids found in docs/parity/*.md\n' >&2
    return 1
  fi

  # Source files: the three scan sets of the header comment. A tree without one of the
  # roots (a fixture, say) is searched in the roots it has.
  local lib_files test_files all_files roots=()
  [ -d crates ] && roots+=(crates)
  if [ "${#roots[@]}" -eq 0 ]; then
    printf 'parity-check: no crates directory under %s\n' "$root" >&2
    return 1
  fi
  lib_files=$(find "${roots[@]}" -type f -name '*.rs' |
    awk '/^crates\/[^\/]+\/src\// { print }' | sort)
  test_files=$(find "${roots[@]}" -type f -name '*.rs' |
    awk '/^crates\/[^\/]+\/tests\// { print }' | sort)
  all_files=$(printf '%s\n%s\n' "$lib_files" "$test_files" | awk 'NF' | sort -u)
  if [ -z "$lib_files" ]; then
    printf 'parity-check: no crates/*/src/**/*.rs under %s\n' "$root" >&2
    return 1
  fi

  local line file rest lineno id status hastest hascol

  # Malformed tables.
  while IFS=$'\037' read -r tag file lineno id status; do
    [ "$tag" = ERR ] || continue
    report "$file" "$lineno" "$id" "$status"
  done <<EOF
$rows
EOF

  # (a) every docs/parity/<file>.md#<id> reference in crates/*/src resolves.
  local ref ref_file ref_id found
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    # line is "path:lineno:docs/parity/x.md#ID"
    file=${line%%:*}
    rest=${line#*:}
    lineno=${rest%%:*}
    ref=${rest#*:}
    ref_file=${ref%%#*}
    ref_id=${ref#*#}
    if [ ! -f "$ref_file" ]; then
      report "$file" "$lineno" "$ref_id" "refers to $ref_file, which does not exist"
      continue
    fi
    found=$(printf '%s\n' "$rows" |
      awk -F'\037' -v f="$ref_file" -v id="$ref_id" '$1 == "ROW" && $2 == f && $4 == id { print "y"; exit }')
    if [ -z "$found" ]; then
      report "$file" "$lineno" "$ref_id" "no row with this id in $ref_file"
    fi
  done <<EOF
$(printf '%s\n' "$lib_files" | awk 'NF' | xargs grep -HnoE 'docs/parity/[a-z0-9_-]+\.md#[A-Za-z0-9_-]+' 2>/dev/null || true)
EOF

  # Every id-shaped token in the scan set, once, for check (b).
  tokens_file=$(mktemp)
  printf '%s\n' "$all_files" | awk 'NF' | xargs grep -ohE "$ID_RE" 2>/dev/null | sort -u >"$tokens_file" || true

  # (b) and the test-id check: every implemented or automated-verified row.
  local subject=0
  while IFS=$'\037' read -r tag file lineno id status hastest hascol; do
    [ "$tag" = ROW ] || continue
    case "$status" in
      implemented* | automated-verified*) ;;
      *) continue ;;
    esac
    subject=$((subject + 1))
    if ! grep -qxF "$id" "$tokens_file"; then
      report "$file" "$lineno" "$id" "status '$status' but no # Compatibility section or test in crates/*/src or crates/*/tests names it"
    fi
    if [ "$hastest" != 1 ]; then
      report "$file" "$lineno" "$id" "status '$status' but the test-id column names no backticked test id"
    fi
  done <<EOF
$rows
EOF
  rm -f "$tokens_file"

  if [ "$violations" -gt 0 ]; then
    printf 'parity-check: FAIL, %s finding(s)\n' "$violations" >&2
    return 1
  fi
  printf 'parity-check: ok, %s implemented or automated-verified row(s) referenced, every source reference resolves\n' "$subject"
}

self_test() {
  local base="$script_dir/testdata/parity-check" failures=0 dir name rc
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
      [ -n "${2:-}" ] && [ -d "$2" ] || { echo "parity-check: --root needs a directory" >&2; exit 2; }
      scan "$(cd "$2" && pwd)"
      ;;
    '') scan "$repo_root" ;;
    *) usage >&2; exit 2 ;;
  esac
}

main "$@"
