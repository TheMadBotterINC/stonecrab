#!/usr/bin/env bash
# D23 graph evidence: an absent package is distinct from a broken Cargo invocation.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p target/dependency-trees
for package in once_cell lazy_static log time chrono; do
  output="target/dependency-trees/$package.txt"
  if cargo tree --locked -p stonecrab --all-features --target all -e normal,dev,build -i "$package" > "$output" 2>&1; then
    cat "$output"
  elif grep -Fq 'did not match any packages' "$output"; then
    cat "$output"
  else
    cat "$output" >&2
    exit 1
  fi
done
