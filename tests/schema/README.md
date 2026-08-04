# Vendored JSON schemas

**do not edit by hand.**

Fetched by `scripts/fetch-schemas.sh` on 2026-08-04
`tests/opds2_test.rs` validates every OPDS 2.0 feed against these offline.
Roots, whose `$ref`s pull in every other file here:

- <https://specs.opds.io/schema/feed.schema.json>
- <https://specs.opds.io/schema/publication.schema.json>

prefixing `https://` gives back the `$id` by which the other schemas refer to it.

Fetched 26 files:

```text
readium.org/webpub-manifest/schema/a11y.schema.json
readium.org/webpub-manifest/schema/altIdentifier.schema.json
readium.org/webpub-manifest/schema/article.schema.json
readium.org/webpub-manifest/schema/chapter.schema.json
readium.org/webpub-manifest/schema/collection.schema.json
readium.org/webpub-manifest/schema/contributor.schema.json
readium.org/webpub-manifest/schema/episode.schema.json
readium.org/webpub-manifest/schema/extensions/encryption/properties.schema.json
readium.org/webpub-manifest/schema/extensions/epub/metadata.schema.json
readium.org/webpub-manifest/schema/extensions/epub/properties.schema.json
readium.org/webpub-manifest/schema/issue.schema.json
readium.org/webpub-manifest/schema/language-map.schema.json
readium.org/webpub-manifest/schema/link.schema.json
readium.org/webpub-manifest/schema/metadata.schema.json
readium.org/webpub-manifest/schema/periodical.schema.json
readium.org/webpub-manifest/schema/season.schema.json
readium.org/webpub-manifest/schema/series.schema.json
readium.org/webpub-manifest/schema/storyArc.schema.json
readium.org/webpub-manifest/schema/subcollection.schema.json
readium.org/webpub-manifest/schema/subject.schema.json
readium.org/webpub-manifest/schema/volume.schema.json
specs.opds.io/schema/acquisition-object.schema.json
specs.opds.io/schema/feed-metadata.schema.json
specs.opds.io/schema/feed.schema.json
specs.opds.io/schema/properties.schema.json
specs.opds.io/schema/publication.schema.json
```

