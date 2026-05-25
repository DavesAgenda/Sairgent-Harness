#!/bin/zsh
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: tools/sqlite-snapshot.sh <source.sqlite> [output.sqlite]" >&2
  exit 1
fi

SOURCE_DB="$1"
OUTPUT_DB="${2:-/tmp/$(basename "${SOURCE_DB%.sqlite}")_snapshot.sqlite}"

if [[ ! -f "$SOURCE_DB" ]]; then
  echo "Source database not found: $SOURCE_DB" >&2
  exit 1
fi

mkdir -p "$(dirname "$OUTPUT_DB")"
rm -f "$OUTPUT_DB"

sqlite3 "$SOURCE_DB" ".timeout 5000" ".backup $OUTPUT_DB"

echo "$OUTPUT_DB"
