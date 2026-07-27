#!/bin/sh

VERSION=$(awk -F ' = ' '$1 ~ /version/ { gsub(/"/, "", $2); printf("%s",$2) }' Cargo.toml)

# prevent Cargo.lock from pointing to pervious version
cargo update --workspace --offline >/dev/null 2>&1 || cargo check --quiet
git diff --quiet HEAD -- Cargo.lock || { echo "Cargo.lock out of date — commit it first" >&2; exit 1; }

if git tag -l | grep -q -x "v${VERSION}"; then
    echo "Tag \"v${VERSION}\" already exists"
else
    echo "Creating tag v${VERSION}"
    git tag -a v${VERSION} -m "Release v${VERSION}"
fi

