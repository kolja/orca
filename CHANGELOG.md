# Changelog

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

Earlier releases: see `git log`.
