#!/usr/bin/env bash
# Clone the seventeen repositories the replay harnesses measure against.
#
# The numbers in STATUS.md come from real code rather than fixtures, and a
# fixture cannot produce the defects these found: a naming rule from one name
# repeated, an extractor blind to a decorated Python class or a generic Rust
# type, a rule counted in one directory refusing a file in another.
#
# Shallow clones, because only the working tree is read. About 250 MB.
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
"

echo "${REPOS}" | while read -r slug name; do
    [ -n "${slug:-}" ] || continue
    if [ -d "${DEST}/${name}" ]; then
        echo "have ${name}"
        continue
    fi
    echo "cloning ${name}"
    git clone --depth 1 -q "https://github.com/${slug}.git" "${DEST}/${name}"
done

echo
echo "corpus ready at ${DEST}"
echo "  python3 tests/replay-tracked-files.py ./target/release/canon $(dirname "${DEST}")"
echo "  python3 tests/replay-new-files.py     ./target/release/canon $(dirname "${DEST}")"
