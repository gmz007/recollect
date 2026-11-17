#!/usr/bin/env bash
# Runs a command (clippy, the tests) and, if it fails, turns what went wrong into GitHub
# annotations, so the run page (and the API, without signing in) says what failed:
# failing tests, panic messages and compiler errors; the end of the output if none of those
# is recognised. Portable to the bash, sed and awk of Linux, macOS and Git for Windows.
#
#   .github/scripts/annotate-failures.sh cargo nextest run --no-fail-fast
set -o pipefail
log=$(mktemp)
if "$@" 2>&1 | tee "$log"; then
  exit 0
fi

annotate() { sed -e 's/%/%25/g' -e 's/^/::error::/'; }

found=$(
  # A panic and the message on the line after it.
  grep -E -A 1 'panicked at' "$log" | grep -v -- '^--$' | paste -d ' ' - - |
    sed -E 's/[[:space:]]+/ /g; s/^ //' | sort -u | head -10
  # nextest: "        FAIL [   0.05s] (12/300) recollect some::test"
  grep -E '^[[:space:]]+(FAIL|SIGSEGV|SIGABRT|TIMEOUT)' "$log" | sed -E 's/^[[:space:]]+//' |
    sort -u | head -10
  # rustc and clippy: "error: ..." followed by "  --> file:line:col".
  awk '/^error(\[[A-Z0-9]+\])?: / { msg = $0; if ((getline loc) > 0) {
         sub(/^[[:space:]]*--> */, "", loc); print loc ": " msg } }' "$log" | head -10
)
# GitHub shows at most 10 error annotations per step: panics (test and reason) come first.
if [ -n "$found" ]; then
  printf '%s\n' "$found" | head -10 | annotate
else
  echo "::error::$(tail -n 20 "$log" | tr '\n' ' ' | sed -e 's/%/%25/g')"
fi
exit 1
