#!/usr/bin/env bash
# The test that matters.
#
# Feeds the binary malformed, truncated, hostile and oversized payloads and
# asserts one contract for every hook subcommand:
#
#   exit code 0, and stdout is either empty or one valid JSON document.
#
# A convention engine that occasionally says nothing is an inconvenience. One
# that panics mid-session gets uninstalled and never reinstalled.
set -uo pipefail

CANON="${1:-./target/release/canon}"
if [ ! -x "${CANON}" ]; then
    echo "usage: $0 <path-to-canon>   (not executable: ${CANON})" >&2
    exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT
export CANON_DATA_DIR="${WORK}/data"

pass=0
fail=0

check() {
    local name="$1" subcommand="$2" payload="$3"
    local out status
    out="$(printf '%s' "${payload}" | "${CANON}" "${subcommand}" 2>"${WORK}/stderr")"
    status=$?

    if [ "${status}" -ne 0 ]; then
        echo "FAIL ${subcommand}/${name}: exit ${status}, expected 0"
        fail=$((fail + 1))
        return
    fi
    if [ -s "${WORK}/stderr" ]; then
        echo "FAIL ${subcommand}/${name}: wrote to stderr, which PostToolUse feeds to the model"
        fail=$((fail + 1))
        return
    fi
    if [ -n "${out}" ] && ! printf '%s' "${out}" | python3 -c 'import json,sys; json.load(sys.stdin)' 2>/dev/null; then
        echo "FAIL ${subcommand}/${name}: stdout is not valid JSON: ${out:0:120}"
        fail=$((fail + 1))
        return
    fi
    pass=$((pass + 1))
}

# Each of these has a different cause and all of them happen.
declare -a NAMES=(
    "empty"
    "not-json"
    "truncated-json"
    "json-array"
    "json-scalar"
    "null-fields"
    "wrong-types"
    "missing-tool-input"
    "path-outside-repo"
    "path-traversal"
    "nul-bytes"
    "deep-nesting"
    "huge-payload"
    "unicode-path"
)
declare -a PAYLOADS=(
    ''
    'this is not json at all'
    '{"hook_event_name":"PreToolUse","tool_input":{"file_path":'
    '[1,2,3]'
    '"just a string"'
    '{"session_id":null,"cwd":null,"tool_input":null}'
    '{"session_id":123,"cwd":[],"tool_input":{"file_path":42}}'
    '{"hook_event_name":"PreToolUse","tool_name":"Write"}'
    '{"cwd":"/tmp","tool_input":{"file_path":"/etc/passwd"}}'
    '{"cwd":"/tmp","tool_input":{"file_path":"/tmp/../../../etc/shadow"}}'
    '{"cwd":"/tmp","tool_input":{"file_path":"/tmp/a\u0000b.rb"}}'
    '{"a":{"a":{"a":{"a":{"a":{"a":{"a":{"a":{"a":{"a":{"a":1}}}}}}}}}}}'
    ''
    '{"cwd":"/tmp","tool_input":{"file_path":"/tmp/日本語/ファイル.rb"}}'
)
# Built here rather than inline so the file stays readable.
PAYLOADS[12]="{\"cwd\":\"/tmp\",\"tool_input\":{\"file_path\":\"/tmp/a.rb\",\"content\":\"$(head -c 200000 /dev/zero | tr '\0' 'x')\"}}"

for subcommand in session-start subagent-start inject verify reconcile; do
    for i in "${!NAMES[@]}"; do
        check "${NAMES[$i]}" "${subcommand}" "${PAYLOADS[$i]}"
    done
done

# No stdin at all is different from empty stdin: the read itself can fail.
for subcommand in session-start subagent-start inject verify reconcile; do
    out="$("${CANON}" "${subcommand}" </dev/null 2>"${WORK}/stderr")"
    status=$?
    if [ "${status}" -eq 0 ] && [ ! -s "${WORK}/stderr" ]; then
        pass=$((pass + 1))
    else
        echo "FAIL ${subcommand}/closed-stdin: exit ${status}"
        fail=$((fail + 1))
    fi
done

total=$((pass + fail))
echo "${pass}/${total} fail-open checks passed"
[ "${fail}" -eq 0 ]
