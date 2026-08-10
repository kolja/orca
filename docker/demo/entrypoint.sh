#!/bin/sh

# ENTRYPOINT with no CMD, so `docker run ... koljaw/orca-demo --auth` arrives
# here as $1 instead of replacing the command.

set -e

banner() {
    printf '\n  Orca %s demo\n' "$ORCA_VERSION"
    printf '  7 public-domain books, in three languages, as EPUB, MOBI and AZW3\n\n'
    printf '    http://127.0.0.1:23117\n'
    printf '    authentication: %s\n\n' "$1"
}

case "$1" in
    --auth)
        ORCA_CONFIG=/demo/auth.toml
        banner "on — log in as alice / secretpassword"
        ;;
    "")
        ORCA_CONFIG=/demo/open.toml
        banner "off — restart with --auth to require a login"
        ;;
    *)
        # --hash, --version, anything else Orca understands: hand it over untouched.
        exec /app/orca "$@"
        ;;
esac

export ORCA_CONFIG
exec /app/orca
