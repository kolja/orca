#!/usr/bin/env bash
#
# Vendor the OPDS 2.0 schemas into tests/schema, so `cargo test` can validate
# feeds without reaching the network. The two OPDS schemas pull in a dozen
# Readium ones, so every "$ref" is followed and fetched too.
#
# Files are stored under the path of their URL -- tests/schema/specs.opds.io/...
# -- which is how tests/opds2_test.rs turns them back into the "$id" each schema
# refers to the others by.

set -euo pipefail

dest="$(cd "$(dirname "$0")/.." && pwd)/tests/schema"

rm -rf "$dest"

fetch() {
    local url=$1
    local file="$dest/${url#https://}"
    if [ -f "$file" ]; then
        return 0
    fi

    echo "$url"
    mkdir -p "$(dirname "$file")"
    curl -sSf "$url" -o "$file"

    # `grep` finding nothing is a schema that refers to no other, not an error.
    { grep -o '"\$ref"[[:space:]]*:[[:space:]]*"[^"#]*' "$file" || true; } | sed 's/.*"//' | sort -u |
        while read -r ref; do
            case "$ref" in
                "") ;;
                https://*) fetch "$ref" ;;
                *) fetch "${url%/*}/$ref" ;;
            esac
        done
}

roots=(
    https://specs.opds.io/schema/feed.schema.json
    https://specs.opds.io/schema/publication.schema.json
)

for root in "${roots[@]}"; do
    fetch "$root"
done

# So that the vendored tree explains itself in a diff.
cat > "$dest/README.md" <<EOF
# Vendored JSON schemas

**do not edit by hand.**

Fetched by \`scripts/fetch-schemas.sh\` on $(date +%Y-%m-%d)
\`tests/opds2_test.rs\` validates every OPDS 2.0 feed against these offline.
Roots, whose \`\$ref\`s pull in every other file here:

$(printf -- '- <%s>\n' "${roots[@]}")

prefixing \`https://\` gives back the \`\$id\` by which the other schemas refer to it.

Fetched $(find "$dest" -name '*.json' | wc -l) files:

\`\`\`text
$(cd "$dest" && find . -name '*.json' | sed 's|^\./||' | sort)
\`\`\`

EOF
