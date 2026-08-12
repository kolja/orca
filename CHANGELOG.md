# Changelog

## 0.2.1

- `publisher` in an OPDS 2.0 publication is an array of objects rather than a
string, the same shape `author` has
- browse by series (`/v2/{lib}/series`) and by publisher (`/v2/{lib}/publishers`)
- `belongsTo.series` carries a link to the rest of the series, and an identifier
- one-line orca demo

## 0.2.0

**existing configs have to change:** A library is a table now, not a path:

```toml
# 0.1                            # 0.2
[calibre.libraries]              [calibre.libraries.library]
library = "/mnt/library"         path = "/mnt/library"
```

Orca refuses to start on the old form.

Also breaking: `v2`, `health` and `auth` are now reserved routes.

New:

- OPDS 2.0 feeds under `/v2`, beside the Atom feeds, which are unchanged
- search, at `/v2/{lib}/search?query=`, offered by every feed of a library
- an OPDS Authentication Document at `/auth`, and as the body of every 401
- a config file that exists but does not parse is an error (not a reason to keep looking for a valid one).

Earlier releases: see `git log`.
