
//! Reading from Calibre
//! SQL statements live here, so can be shared by OPDS 1.2 and 2.0 templates

use html2text::from_read;
use isolang::Language;
use rusqlite::{params, params_from_iter, Connection};
use serde_derive::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

/// How many books are listed in the "Recently Added" category.
const RECENTLY_ADDED: usize = 50;

pub const SYNOPSIS_WIDTH: usize = 100;

/// no blurb will wrap at this width
pub const UNWRAPPED: usize = 10_000;

/// Calibre's own default for a book that has never been touched.
const NEVER_MODIFIED: &str = "2000-01-01 00:00:00+00:00";

#[derive(Debug, Serialize)]
pub struct Book {
    pub id: i32,
    pub uuid: String,
    pub title: String,
    pub pubdate: String,
    pub updated: String,
    /// Calibre's comment, as the HTML it is stored as. `plain_text` renders it.
    pub synopsis: String,
    pub has_cover: bool,
    /// epub, azw3, cbz ...
    pub formats: Vec<String>,
    pub authors: Vec<Author>,
    /// BCP47 tags
    pub languages: Vec<String>,
    pub tags: Vec<Tag>,
    pub publisher: Option<String>,
    pub series: Option<Series>,
}

#[derive(Debug, Serialize)]
pub struct Author {
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct Tag {
    pub id: i32,
    pub name: String,
}

/// The series a book is part of. Calibre lets a book belong to one at most.
#[derive(Debug, Serialize)]
pub struct Series {
    pub name: String,
    /// Calibre's `series_index`: volume in series (could also be 1.5)
    pub index: f64,
}

/// Calibre keeps a blurb as HTML, which neither Atom's `<content type="text">`
/// nor OPDS 2.0's `description` will take. Rendering is left to the caller.
pub fn plain_text(html: &str, width: usize) -> String {
    from_read(html.as_bytes(), width).unwrap_or_default()
}

/// Calibre keeps book formats as CSV. All are offered for download
/// no matter if the mime-type is known to orca or not.
fn parse_formats(concatenated: &str) -> Vec<String> {
    concatenated
        .split(',')
        .filter(|format| !format.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// The mime type of a Calibre format, as `parse_formats` spells it.
pub fn mime(format: &str) -> &'static str {
    match format {
        "epub" => "application/epub+zip",
        "pdf" => "application/pdf",
        "mobi" | "prc" => "application/x-mobipocket-ebook",
        "azw" => "application/vnd.amazon.ebook",
        "azw3" => "application/vnd.amazon.mobi8-ebook",
        "cbz" => "application/vnd.comicbook+zip",
        "cbr" => "application/vnd.comicbook-rar",
        "fb2" => "application/x-fictionbook+xml",
        "djvu" => "image/vnd.djvu",
        "rtf" => "application/rtf",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// A library's connection, recovered from a panic in another handler: a poisoned
/// mutex still guards a perfectly good read-only SQLite connection.
pub fn lock(db: &Mutex<Connection>) -> MutexGuard<'_, Connection> {
    db.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Calibre timestamps look like this: `2024-12-30 14:13:52.213388+00:00`.
/// RFC 4287 §3.3 and RFC 3339 both want date and time separated by 'T'.
fn to_rfc3339(calibre: &str) -> String {
    calibre.replacen(' ', "T", 1)
}

/// "deu" (ISO 639-2) -> "de" (BCP 47)
/// A language that has no two-letter code -- or made up codes -- are passed through unchanged.
fn bcp47(calibre: &str) -> String {
    Language::from_639_3(calibre)
        .and_then(|language| language.to_639_1())
        .unwrap_or(calibre)
        .to_string()
}

/// When the library as a whole last changed.
pub fn updated(db: &Connection) -> String {
    let latest: Option<String> = db
        .query_row("SELECT MAX(last_modified) FROM books;", params![], |row| {
            row.get(0)
        })
        .unwrap_or(None);
    to_rfc3339(&latest.unwrap_or_else(|| NEVER_MODIFIED.to_string()))
}

/// Collect the rows that could be read, logging the ones that could not.
fn collect_rows<T>(rows: impl Iterator<Item = rusqlite::Result<T>>, what: &str) -> Vec<T> {
    rows.filter_map(|row| match row {
        Ok(value) => Some(value),
        Err(e) => {
            eprintln!("Skipping unreadable {} row: {}", what, e);
            None
        }
    })
    .collect()
}

pub fn authors(db: &Connection) -> rusqlite::Result<Vec<Author>> {
    let mut stmt = db.prepare("SELECT id, name FROM authors;")?;
    let rows = stmt.query_map(params![], |row| {
        Ok(Author {
            id: row.get(0)?,
            name: row.get(1)?,
        })
    })?;
    Ok(collect_rows(rows, "author"))
}

pub fn tags(db: &Connection) -> rusqlite::Result<Vec<Tag>> {
    let mut stmt = db.prepare("SELECT id, name FROM tags;")?;
    let rows = stmt.query_map(params![], |row| {
        Ok(Tag {
            id: row.get(0)?,
            name: row.get(1)?,
        })
    })?;
    Ok(collect_rows(rows, "tag"))
}

pub fn books(db: &Connection) -> rusqlite::Result<Vec<Book>> {
    query_books(
        db,
        &format!(
            "SELECT {}
                FROM books b
                LEFT JOIN comments c ON b.id = c.book;",
            BOOK_COLUMNS
        ),
        params![],
    )
}

/// One page of the library, ordered by the sort title Calibre keeps for this purpose.
pub fn books_page(db: &Connection, limit: usize, offset: usize) -> rusqlite::Result<Vec<Book>> {
    query_books(
        db,
        &format!(
            "SELECT {}
                FROM books b
                LEFT JOIN comments c ON b.id = c.book
                ORDER BY b.sort LIMIT ?1 OFFSET ?2;",
            BOOK_COLUMNS
        ),
        params![limit as i64, offset as i64],
    )
}

/// How many books the library holds, for the `numberOfItems` of a feed.
pub fn count_books(db: &Connection) -> rusqlite::Result<usize> {
    let total: i64 = db.query_row("SELECT COUNT(*) FROM books;", params![], |row| row.get(0))?;
    Ok(total as usize)
}

/// A single book, or `QueryReturnedNoRows` for one this library does not hold.
pub fn book(db: &Connection, id: i32) -> rusqlite::Result<Book> {
    let books = query_books(
        db,
        &format!(
            "SELECT {}
                FROM books b
                LEFT JOIN comments c ON b.id = c.book
                WHERE b.id = ?1;",
            BOOK_COLUMNS
        ),
        params![id],
    )?;
    books
        .into_iter()
        .next()
        .ok_or(rusqlite::Error::QueryReturnedNoRows)
}

/// `timestamp`: when a book entered the library. Sorting by `last_modified`
/// would instead show all the books that were just retagged.
pub fn recently_added(db: &Connection) -> rusqlite::Result<Vec<Book>> {
    query_books(
        db,
        &format!(
            "SELECT {}
                FROM books b
                LEFT JOIN comments c ON b.id = c.book
                ORDER BY b.timestamp DESC LIMIT {};",
            BOOK_COLUMNS, RECENTLY_ADDED
        ),
        params![],
    )
}

pub fn books_by_tag(db: &Connection, tag: i32) -> rusqlite::Result<Vec<Book>> {
    query_books(
        db,
        &format!(
            "SELECT {}
                FROM books b
                JOIN books_tags_link bt ON b.id = bt.book
                LEFT JOIN comments c ON b.id = c.book
                WHERE bt.tag = ?1 GROUP BY b.id;",
            BOOK_COLUMNS
        ),
        params![tag],
    )
}

pub fn books_by_author(db: &Connection, author: i32) -> rusqlite::Result<Vec<Book>> {
    query_books(
        db,
        &format!(
            "SELECT {}
                FROM books b
                JOIN books_authors_link ba ON b.id = ba.book
                LEFT JOIN comments c ON b.id = c.book
                WHERE ba.author = ?1 GROUP BY b.id;",
            BOOK_COLUMNS
        ),
        params![author],
    )
}

/// Where a book's cover lives, relative to the library directory.
/// `QueryReturnedNoRows` means either no such book or no cover for it.
pub fn cover_path(db: &Connection, book: i32) -> rusqlite::Result<String> {
    let mut stmt =
        db.prepare("SELECT books.path FROM books WHERE books.id = ?1 AND books.has_cover = true;")?;
    let path: String = stmt.query_row(params![book], |row| row.get(0))?;
    Ok(format!("{}/cover.jpg", path))
}

/// Where one format of a book lives, relative to the library directory.
/// Calibre files every format of a book under the same stem, so the format only
/// decides the extension.
pub fn file_path(db: &Connection, book: i32, format: &str) -> rusqlite::Result<String> {
    let mut stmt = db.prepare(
        "SELECT b.path, d.name AS file
            FROM books b
            JOIN data d ON b.id = d.book
            WHERE b.id = ?1 AND d.format = ?2 COLLATE NOCASE;",
    )?;
    let (path, file): (String, String) =
        stmt.query_row(params![book, format], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(format!("{}/{}.{}", path, file, format))
}

/// The columns every book feed needs.
/// `GROUP_CONCAT` : a client should not see the download links shuffle between requests
const BOOK_COLUMNS: &str = "b.id, b.uuid, b.title, b.pubdate, b.last_modified, b.has_cover, b.series_index, c.text AS synopsis,
    (SELECT GROUP_CONCAT(format)
        FROM (SELECT format FROM data WHERE book = b.id ORDER BY format)) AS formats,
    (SELECT s.name FROM books_series_link bs JOIN series s ON bs.series = s.id
        WHERE bs.book = b.id) AS series,
    (SELECT p.name FROM books_publishers_link bp JOIN publishers p ON bp.publisher = p.id
        WHERE bp.book = b.id) AS publisher";

/// Run one of the book queries and map its rows to `Book`s.
fn query_books(
    db: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> rusqlite::Result<Vec<Book>> {
    let mut stmt = db.prepare(sql)?;
    let rows = stmt.query_map(params, |row| {
        let formats = parse_formats(&row.get::<_, String>("formats").unwrap_or_default());
        // no `series_intex` if the book is not part of a series
        let series = row
            .get::<_, Option<String>>("series")
            .unwrap_or(None)
            .map(|name| Series {
                name,
                index: row.get("series_index").unwrap_or(1.0),
            });
        Ok(Book {
            id: row.get("id")?,
            uuid: row.get("uuid").unwrap_or_default(),
            title: row.get("title")?,
            pubdate: to_rfc3339(&row.get::<_, String>("pubdate")?),
            updated: to_rfc3339(&row.get::<_, String>("last_modified")?),
            synopsis: row.get("synopsis").unwrap_or_default(),
            has_cover: row.get("has_cover").unwrap_or(false),
            formats,
            authors: Vec::new(),
            languages: Vec::new(),
            tags: Vec::new(),
            publisher: row.get("publisher").unwrap_or(None),
            series,
        })
    })?;

    let mut books = collect_rows(rows, "book");
    let book_ids: Vec<i32> = books.iter().map(|book| book.id).collect();
    let mut authors = authors_by_book(db, &book_ids)?;
    let mut languages = languages_by_book(db, &book_ids)?;
    let mut tags = tags_by_book(db, &book_ids)?;
    for book in &mut books {
        book.authors = authors.remove(&book.id).unwrap_or_default();
        book.languages = languages.remove(&book.id).unwrap_or_default();
        book.tags = tags.remove(&book.id).unwrap_or_default();
    }
    Ok(books)
}

/// The authors of each of the given books, by book id.
/// Books are joined to their authors in a separate query, so that a co-authored
/// book stays one row rather than one row per author.
fn authors_by_book(db: &Connection, book_ids: &[i32]) -> rusqlite::Result<HashMap<i32, Vec<Author>>> {
    if book_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut stmt = db.prepare(&format!(
        "SELECT ba.book, a.id, a.name
            FROM books_authors_link ba
            JOIN authors a ON ba.author = a.id
            WHERE ba.book IN ({})
            ORDER BY ba.book, a.sort;",
        placeholders(book_ids.len())
    ))?;

    let rows = stmt.query_map(params_from_iter(book_ids), |row| {
        Ok((
            row.get::<_, i32>(0)?,
            Author {
                id: row.get(1)?,
                name: row.get(2)?,
            },
        ))
    })?;

    Ok(group_by_book(collect_rows(rows, "book author")))
}

/// The tags of each of the given books, by book id.
fn tags_by_book(db: &Connection, book_ids: &[i32]) -> rusqlite::Result<HashMap<i32, Vec<Tag>>> {
    if book_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut stmt = db.prepare(&format!(
        "SELECT bt.book, t.id, t.name
            FROM books_tags_link bt
            JOIN tags t ON bt.tag = t.id
            WHERE bt.book IN ({})
            ORDER BY bt.book, t.name;",
        placeholders(book_ids.len())
    ))?;

    let rows = stmt.query_map(params_from_iter(book_ids), |row| {
        Ok((
            row.get::<_, i32>(0)?,
            Tag {
                id: row.get(1)?,
                name: row.get(2)?,
            },
        ))
    })?;

    Ok(group_by_book(collect_rows(rows, "book tag")))
}

/// The languages of each of the given books, by book id.
fn languages_by_book(db: &Connection, book_ids: &[i32]) -> rusqlite::Result<HashMap<i32, Vec<String>>> {
    if book_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut stmt = db.prepare(&format!(
        "SELECT bl.book, l.lang_code
            FROM books_languages_link bl
            JOIN languages l ON bl.lang_code = l.id
            WHERE bl.book IN ({})
            ORDER BY bl.book, bl.item_order;",
        placeholders(book_ids.len())
    ))?;

    let rows = stmt.query_map(params_from_iter(book_ids), |row| {
        Ok((row.get::<_, i32>(0)?, bcp47(&row.get::<_, String>(1)?)))
    })?;

    Ok(group_by_book(collect_rows(rows, "book language")))
}

fn placeholders(count: usize) -> String {
    vec!["?"; count].join(",")
}

fn group_by_book<T>(rows: Vec<(i32, T)>) -> HashMap<i32, Vec<T>> {
    let mut by_book: HashMap<i32, Vec<T>> = HashMap::new();
    for (book, value) in rows {
        by_book.entry(book).or_default().push(value);
    }
    by_book
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> Connection {
        Connection::open("tests/calibre/metadata.db").expect("test library")
    }

    #[test]
    fn calibre_dates_become_rfc3339() {
        assert_eq!(
            to_rfc3339("2024-12-30 14:13:52.213388+00:00"),
            "2024-12-30T14:13:52.213388+00:00"
        );
        // Only the separator moves; an already-correct date is left alone.
        assert_eq!(to_rfc3339("2024-12-30T14:13:52+00:00"), "2024-12-30T14:13:52+00:00");
    }

    // Calibre writes ISO 639-2; OPDS clients read BCP 47.
    #[test]
    fn three_letter_languages_shrink_to_two() {
        assert_eq!(bcp47("deu"), "de");
        assert_eq!(bcp47("eng"), "en");
        assert_eq!(bcp47("rus"), "ru");
    }

    // Not every language has a two-letter code, and Calibre lets you type
    // anything into the field.
    #[test]
    fn languages_without_a_short_code_are_left_alone() {
        assert_eq!(bcp47("haw"), "haw");
        assert_eq!(bcp47("foobar"), "foobar");
    }

    #[test]
    fn books_carry_their_language() {
        let db = library();
        let books = books(&db).expect("books");

        let kant = books.iter().find(|book| book.id == 5).expect("Kant");
        assert_eq!(kant.languages, ["de"]);

        let tolstoy = books.iter().find(|book| book.id == 2).expect("Tolstoy");
        assert_eq!(tolstoy.languages, ["ru"]);
    }

    // Dropping a format Orca has no mime type for would leave a book that is
    // only stored as azw3 with nothing to download at all.
    #[test]
    fn every_format_is_offered_however_exotic() {
        assert_eq!(parse_formats("EPUB,AZW3,CBZ"), ["epub", "azw3", "cbz"]);
        assert_eq!(mime("azw3"), "application/vnd.amazon.mobi8-ebook");
        assert_eq!(mime("epub"), "application/epub+zip");
        // What Orca cannot name, the client may still know what to do with.
        assert_eq!(mime("foo"), "application/octet-stream");
    }

    // GROUP_CONCAT over no rows is NULL, which reads back as "".
    #[test]
    fn a_book_with_no_files_has_no_formats() {
        assert!(parse_formats("").is_empty());
    }

    #[test]
    fn books_carry_every_format_the_library_holds() {
        let db = library();
        let books = books(&db).expect("books");

        let alice = books.iter().find(|book| book.id == 4).expect("Alice");
        assert_eq!(alice.formats, ["azw3", "epub"]);
    }

    // Every page has to be a window on the same order, so that paging through
    // the catalog shows every book exactly once.
    #[test]
    fn pages_follow_calibres_sort_title() {
        let db = library();
        let ids = |books: Vec<Book>| books.iter().map(|book| book.id).collect::<Vec<_>>();

        assert_eq!(count_books(&db).expect("count"), 7);
        assert_eq!(ids(books_page(&db, 2, 0).expect("first page")), [4, 8]);
        assert_eq!(ids(books_page(&db, 2, 2).expect("second page")), [9, 5]);
        // Kant sorts under K, but Galileo under "sidereal messenger, The".
        assert_eq!(ids(books_page(&db, 2, 4).expect("third page")), [6, 7]);
        // Seven books, pages of two: the last one holds the remainder.
        assert_eq!(ids(books_page(&db, 2, 6).expect("last page")), [2]);
        assert!(books_page(&db, 10, 7).expect("past the end").is_empty());
    }

    #[test]
    fn a_single_book_reads_like_one_out_of_a_feed() {
        let db = library();
        let alice = book(&db, 4).expect("Alice");

        assert_eq!(alice.title, "Alice's Adventures in Wonderland");
        assert_eq!(alice.formats, ["azw3", "epub"]);
        assert_eq!(alice.authors[0].name, "Lewis Carroll");
        assert!(alice.has_cover);
    }

    #[test]
    fn a_book_carries_the_shelf_it_came_off() {
        let db = library();
        let patrol = book(&db, 9).expect("Galactic Patrol");

        let series = patrol.series.expect("a series");
        assert_eq!(series.name, "Astounding Stories");
        assert_eq!(series.index, 3.0);
        assert_eq!(patrol.publisher.as_deref(), Some("Street & Smith"));

        let tags: Vec<&str> = patrol.tags.iter().map(|tag| tag.name.as_str()).collect();
        assert_eq!(tags, ["science fiction", "space opera"]);
    }

    // Calibre keeps a `series_index` of 1.0 for every book, series or not, so
    // only the series itself can say whether the number means anything.
    #[test]
    fn a_book_in_no_series_is_in_no_series() {
        let db = library();
        assert!(book(&db, 4).expect("Alice").series.is_none());
    }

    // Neither feed can use the HTML Calibre stores, and the two want it
    // rendered differently, so a book carries it as it was written.
    #[test]
    fn a_blurb_leaves_calibre_as_html() {
        let db = library();
        let alice = book(&db, 4).expect("Alice");
        assert!(alice.synopsis.starts_with("<div>"));

        let wrapped = plain_text(&alice.synopsis, SYNOPSIS_WIDTH);
        assert!(!wrapped.contains("<div>"));
        assert!(wrapped.lines().all(|line| line.chars().count() <= SYNOPSIS_WIDTH));

        // The same blurb, without the breaks the 100 columns forced into it.
        let unwrapped = plain_text(&alice.synopsis, UNWRAPPED);
        assert!(unwrapped.lines().count() < wrapped.lines().count());
    }

    #[test]
    fn a_book_the_library_does_not_hold_is_no_rows() {
        let db = library();
        assert!(matches!(
            book(&db, 99999),
            Err(rusqlite::Error::QueryReturnedNoRows)
        ));
    }

    #[test]
    fn a_missing_cover_is_no_rows_rather_than_an_error() {
        let db = library();
        assert!(matches!(
            cover_path(&db, 99999),
            Err(rusqlite::Error::QueryReturnedNoRows)
        ));
    }

    #[test]
    fn paths_are_relative_to_the_library() {
        let db = library();
        assert!(cover_path(&db, 5).expect("cover").ends_with("/cover.jpg"));
        assert!(file_path(&db, 5, "epub").expect("file").ends_with(".epub"));
        // Calibre spells its formats in upper case, the routes in lower.
        assert!(file_path(&db, 4, "azw3").expect("file").ends_with(".azw3"));
    }

    // Kant has an epub only. Asking for a pdf -> 404
    #[test]
    fn a_format_the_library_does_not_hold_is_no_rows() {
        let db = library();
        assert!(matches!(
            file_path(&db, 5, "pdf"),
            Err(rusqlite::Error::QueryReturnedNoRows)
        ));
    }

    // An empty library must not produce `... IN ()`.
    #[test]
    fn a_feed_with_no_books_asks_for_no_authors() {
        let db = library();
        assert!(books_by_tag(&db, 99999).expect("no books").is_empty());
    }
}
