#!/usr/bin/env bash
# Usage: scripts/dco-check.sh [RANGE]
#
# Fails when any commit in the git revision RANGE has no `Signed-off-by:` trailer
# (CODING_STANDARDS section 18.3). Offending commits are listed as "<hash> <subject>".
# RANGE defaults to origin/master..HEAD. Examples:
#   scripts/dco-check.sh c825b9d..HEAD
#   scripts/dco-check.sh "$BASE_SHA..$HEAD_SHA"          # pull_request event
#   scripts/dco-check.sh "$BEFORE_SHA..$GITHUB_SHA"      # push event
# An empty range passes. The root commit 97a5a2d predates the DCO policy and is never
# in a base..head range, so it is not special-cased here.
#
# Exit status: 0 all signed, 1 unsigned commit(s) found, 2 usage or git error.
# Needs git 2.22+ for %(trailers:key=...); portable bash 3.2+ (macOS, Linux, Git Bash).
set -euo pipefail

case "${1:-}" in
  -h | --help)
    sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
esac

if [ "$#" -gt 1 ]; then
  echo "dco-check: expected at most one RANGE argument" >&2
  exit 2
fi
range=${1:-origin/master..HEAD}

if ! commits=$(git rev-list "$range" 2>&1); then
  printf 'dco-check: cannot resolve range %s: %s\n' "$range" "$commits" >&2
  exit 2
fi

total=0
bad=0
for sha in $commits; do
  total=$((total + 1))
  trailer=$(git log -1 --format='%(trailers:key=Signed-off-by,valueonly)' "$sha")
  if ! printf '%s\n' "$trailer" | grep -Eq '^[^<>]+ <[^<>@ ]+@[^<>@ ]+>[[:space:]]*$'; then
    bad=$((bad + 1))
    printf 'dco-check: missing Signed-off-by: %s\n' "$(git log -1 --format='%h %s' "$sha")" >&2
  fi
done

if [ "$bad" -gt 0 ]; then
  printf 'dco-check: FAIL, %s of %s commit(s) in %s lack Signed-off-by (git commit -s)\n' \
    "$bad" "$total" "$range" >&2
  exit 1
fi
printf 'dco-check: ok, %s commit(s) in %s signed off\n' "$total" "$range"
