#!/bin/sh

VERSION=$(awk -F ' = ' '$1 ~ /version/ { gsub(/[\"]/, "", $2); printf("%s",$2) }' Cargo.toml)

if git tag -l | grep -q -x "v${VERSION}"; then
    echo "Tag \"v${VERSION}\" already exists"
else
    echo "Creating tag v${VERSION}"
    git tag -a v${VERSION} -m "Release v${VERSION}"
fi

