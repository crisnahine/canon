#!/usr/bin/env bash
# Clone the twenty-four repositories the replay harnesses measure against.
#
# The numbers in STATUS.md come from real code rather than fixtures, and a
# fixture cannot produce the defects these found: a naming rule from one name
# repeated, an extractor blind to a decorated Python class or a generic Rust
# type, a rule counted in one directory refusing a file in another. Seven of
# the twenty-four are applications built on a framework rather than a library
# or the framework itself: a library's own classes rarely inherit through a
# chain of mixins, so a corpus of only libraries can't produce the
# mixin-first base lists that framework applications write everywhere.
#
# Shallow clones, because only the working tree is read. About 600 MB.
#
# Two exceptions. `git log` over a `--depth 1` clone has exactly one commit,
# so every tracked file's "most recent commit" is that same one stamp — the
# "one distinct mtime across 400 files" condition commit-time recency exists
# to fix, reproduced by the corpus itself rather than fixed by it. A shallow
# corpus can never exercise that feature, and nothing downstream would notice
# if it regressed. `DEEP_REPOS` below are cloned with real history instead, and
# `verify_commit_time_coverage` fails the script if that history turns out not
# to vary — the assertion this file exists to add.
set -euo pipefail

DEST="${1:?usage: clone-corpus.sh <directory>}"
mkdir -p "${DEST}"

REPOS="
rubocop/rubocop        ruby-rubocop
sinatra/sinatra        ruby-sinatra
mastodon/mastodon      rb-mastodon
pallets/flask          py-flask
psf/requests           py-requests
spf13/cobra            go-cobra
gin-gonic/gin          go-gin
BurntSushi/ripgrep     rs-ripgrep
clap-rs/clap           rs-clap
serde-rs/serde         rs-serde
tokio-rs/bytes         rs-bytes
slimphp/Slim           php-slim
laravel/framework      php-laravel
vuejs/core             vue-core
nuxt/nuxt              vue-nuxt
reduxjs/redux-toolkit  ts-rtk
axios/axios            js-axios
wagtail/wagtail                          py-wagtail
tiangolo/full-stack-fastapi-template     py-fastapi-app
pixelfed/pixelfed                        php-pixelfed
nestjs/nest                              ts-nest
nuxt/ui                                  vue-nuxt-ui
gohugoio/hugo                            go-hugo
starship/starship                        rs-starship
"

# Deep enough that an active repository's last few hundred commits span many
# distinct days, shallow enough that two repositories cost tens of megabytes
# rather than the full history every replay harness would otherwise have to
# pay for on all twenty-four.
DEEP_CLONE_COMMITS=300

# One small library, one mid-sized one: real, ordinary commit histories
# rather than a repository picked for having one.
DEEP_REPOS="ruby-sinatra rs-ripgrep"

is_deep_repo() {
    local name="$1" candidate
    for candidate in ${DEEP_REPOS}; do
        [ "${candidate}" = "${name}" ] && return 0
    done
    return 1
}

echo "${REPOS}" | while read -r slug name; do
    [ -n "${slug:-}" ] || continue
    if [ -d "${DEST}/${name}" ]; then
        echo "have ${name}"
        continue
    fi
    if is_deep_repo "${name}"; then
        echo "cloning ${name} (${DEEP_CLONE_COMMITS} commits, for commit-time coverage)"
        git clone --depth "${DEEP_CLONE_COMMITS}" -q "https://github.com/${slug}.git" "${DEST}/${name}"
    else
        echo "cloning ${name}"
        git clone --depth 1 -q "https://github.com/${slug}.git" "${DEST}/${name}"
    fi
done

# Below this many distinct commit timestamps, the deep clone above is
# indistinguishable from a shallow one for the purpose it was made deep for.
MIN_DISTINCT_COMMIT_TIMES=20

verify_commit_time_coverage() {
    local name="$1" dir="${DEST}/${name}"
    if [ ! -d "${dir}" ]; then
        echo "commit-time coverage: ${name} is not present, skipping" >&2
        return 0
    fi
    local distinct
    distinct=$(git -C "${dir}" log --format='%ct' | sort -u | wc -l | tr -d ' ')
    echo "commit-time coverage: ${name} has ${distinct} distinct commit timestamps"
    if [ "${distinct}" -lt "${MIN_DISTINCT_COMMIT_TIMES}" ]; then
        echo "error: ${name} has only ${distinct} distinct commit timestamps (need >= ${MIN_DISTINCT_COMMIT_TIMES})." >&2
        echo "the corpus no longer exercises commit-time recency; remove ${dir} and re-run this script, or widen DEEP_CLONE_COMMITS." >&2
        return 1
    fi
}

coverage_failed=0
for name in ${DEEP_REPOS}; do
    verify_commit_time_coverage "${name}" || coverage_failed=1
done
if [ "${coverage_failed}" -ne 0 ]; then
    exit 1
fi

echo
echo "corpus ready at ${DEST}"
echo "  python3 tests/replay-tracked-files.py ./target/release/canon $(dirname "${DEST}")"
echo "  python3 tests/replay-new-files.py     ./target/release/canon $(dirname "${DEST}")"
