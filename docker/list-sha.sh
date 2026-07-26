#!/bin/sh

VERSION=$(awk -F ' = ' '$1 ~ /version/ { gsub(/"/, "", $2); printf("v%s",$2) }' Cargo.toml)

# sha256sum comes with coreutils (Linux), shasum with perl (macOS)
sha256() {
    if command -v sha256sum > /dev/null 2>&1; then
        sha256sum
    else
        shasum -a 256
    fi
}

SHA256_DARWIN_ARM=$(http --download https://github.com/kolja/orca/releases/download/${VERSION}/orca-aarch64-apple-darwin.tar.gz -o - 2>/dev/null | sha256 | awk '{print $1}')
SHA256_LINUX_X86=$(http --download https://github.com/kolja/orca/releases/download/${VERSION}/orca-x86_64-unknown-linux-musl.tar.gz -o - 2>/dev/null | sha256 | awk '{print $1}')

echo "VERSION = \"${VERSION}\""
echo "SHA256_DARWIN_ARM = \"${SHA256_DARWIN_ARM}\""
echo "SHA256_LINUX_X86 = \"${SHA256_LINUX_X86}\""
