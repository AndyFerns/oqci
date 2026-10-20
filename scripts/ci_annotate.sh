#!/usr/bin/env bash
# Run a command; if it fails, publish the tail of its output as a GitHub
# Actions error annotation, then fail with the command's exit code.
#
#   scripts/ci_annotate.sh "<title>" <command> [args...]
#
# Job logs are visible only to signed-in users; annotations are public (the
# check-runs API serves them without authentication), so a failure is
# diagnosable from anywhere. Output is also streamed to the log as usual.
set -uo pipefail

title="$1"
shift
log="$(mktemp)"
"$@" 2>&1 | tee "$log"
status=${PIPESTATUS[0]}
if [ "$status" -ne 0 ]; then
  # Annotations are one line: encode newlines (and the escape characters)
  # per the workflow-command rules. Keep the last 60 lines.
  tail -n 60 "$log" | sed -e 's/%/%25/g' -e 's/\r/%0D/g' | awk 'BEGIN{ORS="%0A"} {print}' > "$log.tail"
  echo "::error title=${title} (exit ${status})::$(cat "$log.tail")"
fi
rm -f "$log" "$log.tail"
exit "$status"
