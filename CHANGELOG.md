# Changelog

## 0.2.2

The library root is a landing page now and can be configured.
See the `README.md` and `landing_page.toml` for details.

`/` now redirects to `/v1` (also a reserved route now) for the Atom feed.
Covers and downloads remain unversioned under `/{lib}/cover/{id}` and `/{lib}/file/{id}/{format}`
Trailing or a doubled slashes are normalized.

*public = []* in the config:

A `public` entry names a library rather than a URL, and globs are gone (but trailing `/**` is still legal).
`/library` makes both formats public, `/v1/library` or `/v2/library` only the version specified.

Leaving out `[authentication]` altogether serves every library without a login.
`[authentication]` present but empty is an error, so is an unknown key anywhere in the config.

`/health` needs no credentials any more
The catalog lists only the libraries the caller can open.

Calibre libraries are opened read-only

Spelling of Protocol in the config: `http` and `https` (all lowercase). 
`Http` and `Https` still work but consider that spelling deprecated.

Some `identifiers` from books in Calibre are exposed to the feed (isbn, doi...)

OPDS 2.0 publications carry `sortAs` for the title, the authors and the series

## 0.2.1

- added a one-line orca demo
- `publisher` in an OPDS 2.0 publication is an array of objects rather than a
string, the same shape `author` has.
- browse by series `{lib}/series` and by publisher `{lib}/publishers` for both OPDS v1 and v2
- `belongsTo.series` carries a link to the rest of the series, and an identifier

**specific to OPDS 1.2 (Atom) feeds:**

- every `href` is absolute now
- paging, by `?page=`, with `first`/`previous`/`next`/`last` links
- search, discoverable via `opensearch.xml`
- an entry comes with tags as `<category>`, and its language, publisher and
publication date as `dcterms:language`, `dcterms:publisher` and `dcterms:issued`
- an acquisition link has `length` now
- an entry links to its series and its publisher with `rel="related"`
- a `<summary>` beside the `<content>`
- `rel="up"` back to the library, in every feed below it
- a shelf feed is titled after its shelf rather than "{lib} | n books"
- the self link's content type says `kind=navigation` or `kind=acquisition`
- a category with no books to it is no longer listed, matching `/v2`

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
