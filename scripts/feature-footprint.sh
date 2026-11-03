#!/usr/bin/env bash
# Linux /usr/bin/time: maximum RSS is a process high-water mark, not aggregate memory.
set -euo pipefail
cd "$(dirname "$0")/.."
export AWS_LC_SYS_USE_SYSTEM=0
mkdir -p target/feature-footprint
build_root=$(mktemp -d "$PWD/target/feature-footprint-build.XXXXXX")
trap 'rm -rf "$build_root"' EXIT
report=target/feature-footprint/report.tsv
printf 'features\tbuild_seconds\tcompiler_max_rss_kib\tregistry_max_rss_kib\tbinary_bytes\tarchive_bytes\n' > "$report"
for features in modbus modbus,opcua modbus,tls,opcua; do
  destination="$build_root/$features"
  /usr/bin/time -f '%e\t%M' -o "target/feature-footprint/$features-build.tsv" \
    cargo build --locked -p stonecrab --release --no-default-features --features "$features" \
      --example feature_footprint --target-dir "$destination"
  binary="$destination/release/examples/feature_footprint"
  /usr/bin/time -f '%M' -o "target/feature-footprint/$features-rss.txt" "$binary"
  binary_bytes=$(stat -c '%s' "$binary")
  archive_bytes=$(find "$destination/release" -type f \( -name '*.rlib' -o -name '*.a' \) -printf '%s\n' | awk '{sum += $1} END {printf "%.0f", sum}')
  printf '%s\t%s\t%s\t%s\t%s\n' "$features" \
    "$(cat "target/feature-footprint/$features-build.tsv")" \
    "$(cat "target/feature-footprint/$features-rss.txt")" "$binary_bytes" "$archive_bytes" >> "$report"
done
cat "$report"
