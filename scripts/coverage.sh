#!/usr/bin/env bash
#
# Run the test suite under cargo-tarpaulin and compare per-file line coverage
# against coverage-baseline.txt, so a drop shows up here instead of on
# coveralls after a push.
#
#   scripts/coverage.sh            check against the baseline, non-zero exit on a drop
#   scripts/coverage.sh --update   accept this run as the new baseline
#
# Per file rather than project-wide on purpose: a whole-project threshold hides
# one file rotting while another improves.

set -euo pipefail

cd "$(dirname "$0")/.."

BASELINE="coverage-baseline.txt"
REPORT_DIR="target/coverage"

if ! command -v cargo-tarpaulin > /dev/null; then
    echo "cargo-tarpaulin is not installed. Run: cargo install cargo-tarpaulin" >&2
    exit 127
fi

mkdir -p "$REPORT_DIR"
cargo tarpaulin --out Lcov --output-dir "$REPORT_DIR" > /dev/null

# lcov gives one "DA:<line>,<hits>" record per instrumented line, grouped by the
# "SF:<path>" that precedes them.
awk -F'[:,]' '
    /^SF:/ { file = $2; sub(/^.*\/orca\//, "", file); next }
    /^DA:/ { total[file]++; if ($3 > 0) covered[file]++ }
    END    { for (f in total) printf "%s %d %d\n", f, covered[f] + 0, total[f] }
' "$REPORT_DIR/lcov.info" | sort > "$REPORT_DIR/current.txt"

if [ "${1:-}" = "--update" ]; then
    cp "$REPORT_DIR/current.txt" "$BASELINE"
    echo "Baseline updated:"
    awk '{ printf "  %-20s %6.2f%% (%d/%d)\n", $1, $2 * 100 / $3, $2, $3 }' "$BASELINE"
    exit 0
fi

if [ ! -f "$BASELINE" ]; then
    echo "No $BASELINE yet. Create one with: scripts/coverage.sh --update" >&2
    exit 1
fi

# A file missing from either side is not a regression: it was added or deleted.
join -a1 -a2 -e- -o 0,1.2,1.3,2.2,2.3 "$BASELINE" "$REPORT_DIR/current.txt" | awk '
    {
        file = $1
        if ($2 == "-") { printf "  %-20s      new  %6.2f%% (%d/%d)\n", file, $4 * 100 / $5, $4, $5; next }
        if ($4 == "-") { printf "  %-20s  removed\n", file; next }

        was = $2 * 100 / $3
        now = $4 * 100 / $5
        delta = now - was

        if (delta < -0.001) {
            printf "  %-20s %6.2f%% -> %6.2f%%  %+.2f  (%d/%d -> %d/%d)  REGRESSED\n", \
                   file, was, now, delta, $2, $3, $4, $5
            failed = 1
        } else {
            printf "  %-20s %6.2f%% -> %6.2f%%  %+.2f\n", file, was, now, delta
        }
    }
    END { exit failed + 0 }
' || {
    echo
    echo "Coverage regressed. Add tests, or accept it with: scripts/coverage.sh --update" >&2
    exit 1
}

echo
echo "No file lost coverage."
