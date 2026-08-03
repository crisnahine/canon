#!/usr/bin/env bash
# Asserts the assumption the whole design rests on.
#
# canon injects conventions from a PreToolUse hook. That only works if
# `hookSpecificOutput.additionalContext` on PreToolUse both reaches the model
# *and* arrives before the tool executes. Neither is promised by the documented
# field list: `permissionDecision` and `updatedInput` are described as
# PreToolUse-only and `additionalContext` is not described as belonging to it
# at all.
#
# So it is checked behaviourally, against the installed host. A hook injects a
# convention the request never mentions, and the resulting file either carries
# it or does not. `Edit` is withheld so the model cannot write the file and fix
# it afterwards: the first Write has to be right.
#
# If this ever fails, canon's per-file injection is silently doing nothing, and
# the delivery point has to move to SessionStart and PostToolUse.
set -uo pipefail

if ! command -v claude >/dev/null 2>&1; then
    echo "SKIP: the claude CLI is not on PATH"
    exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT
mkdir -p "${WORK}/.claude"

MARKER='### CANON-HDR ###'
cat >"${WORK}/.claude/settings.json" <<JSON
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Write",
        "hooks": [
          {
            "type": "command",
            "command": "printf '{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"additionalContext\":\"Conventions for *.txt, derived from 41 files: every .txt file begins with the line \`${MARKER}\` as its first line. (41/41, 1.00)\"}}\\\\n'"
          }
        ]
      }
    ]
  }
}
JSON

(
    cd "${WORK}" || exit 1
    claude -p "Create a file note.txt with a short greeting." \
        --permission-mode acceptEdits \
        --allowedTools "Write" \
        </dev/null >/dev/null 2>&1
)

if [ ! -f "${WORK}/note.txt" ]; then
    echo "INCONCLUSIVE: the model wrote no file; nothing to assert"
    exit 0
fi

if head -1 "${WORK}/note.txt" | grep -qF "${MARKER}"; then
    echo "PASS: PreToolUse additionalContext reached the model and shaped the write"
    exit 0
fi

echo "FAIL: the injected convention did not reach the first Write."
echo "canon's per-file injection is a no-op against this host version."
echo "note.txt was:"
sed 's/^/    /' "${WORK}/note.txt"
exit 1
