#!/usr/bin/env bash
# Every platform the shim can ask for must be a platform the release publishes.
#
# This is the gap that shipped a plugin doing nothing on Windows: the shim
# computed `canon-x86_64-pc-windows-msvc` and the release published
# `canon-x86_64-pc-windows-msvc.exe`. A 404, a silent fail-open, and no symptom
# beyond the plugin never speaking.
#
# The failure mode is structural rather than a typo, so checking it by hand
# after each release is not enough. Both sides are enumerated from source here:
# the shim's `target_triple` from its own case arms, the release's targets from
# the workflow matrix.
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

SHIM="bin/canon"
RELEASE=".github/workflows/release.yml"
fail=0

# Every triple the shim can produce: the cross product of the architectures and
# the platform strings it knows, plus the `.exe` suffix rule for Windows.
shim_triples() {
    local arches os
    arches=$(sed -n 's/.*_arch_part="\([a-z0-9_]*\)".*/\1/p' "${SHIM}" | sort -u)
    os=$(sed -n 's/.*_os_part="\([a-z0-9-]*\)".*/\1/p' "${SHIM}" | sort -u)
    for a in ${arches}; do
        for o in ${os}; do
            # aarch64 musl and gnu both exist; every combination the shim can
            # emit has to be publishable, so enumerate them all.
            case "${o}" in
                pc-windows-msvc) printf 'canon-%s-%s.exe\n' "${a}" "${o}" ;;
                *) printf 'canon-%s-%s\n' "${a}" "${o}" ;;
            esac
        done
    done
}

# Every asset the release workflow produces, from its matrix.
release_assets() {
    local target
    while read -r target; do
        case "${target}" in
            *windows*) printf 'canon-%s.exe\n' "${target}" ;;
            *) printf 'canon-%s\n' "${target}" ;;
        esac
    done < <(sed -n 's/^ *- target: \([a-z0-9_-]*\)$/\1/p' "${RELEASE}" | sort -u)
}

WANTED=$(shim_triples | sort -u)
PUBLISHED=$(release_assets | sort -u)

if [ -z "${WANTED}" ] || [ -z "${PUBLISHED}" ]; then
    echo "FAIL: could not parse triples (shim=$(echo "${WANTED}" | wc -l), release=$(echo "${PUBLISHED}" | wc -l))"
    exit 1
fi

echo "the shim can ask for:"
echo "${WANTED}" | sed 's/^/  /'
echo
echo "the release publishes:"
echo "${PUBLISHED}" | sed 's/^/  /'
echo

while read -r asset; do
    if ! printf '%s\n' "${PUBLISHED}" | grep -qx "${asset}"; then
        echo "FAIL: the shim can request ${asset}, which no release job builds"
        fail=1
    fi
done <<<"${WANTED}"

# The reverse is a warning, not a failure: publishing something unreachable is
# waste rather than breakage, and a target may be added ahead of the detection
# that selects it.
while read -r asset; do
    if ! printf '%s\n' "${WANTED}" | grep -qx "${asset}"; then
        echo "note: ${asset} is published but the shim never asks for it"
    fi
done <<<"${PUBLISHED}"

if [ "${fail}" -eq 0 ]; then
    echo "every platform the shim can ask for is published"
fi
exit "${fail}"
