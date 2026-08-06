//! SQLite storage for Notes: one table, soft deletes so the bus's undo
//! journal can compensate (`delete_note` ↔ `restore_note`).

use rusqlite::{Connection, params};
use std::path::Path;

/// One row as `list_notes` reports it.
///
/// `snippet` is the opening of the body — enough for a list row's
/// subtitle, cut in SQL (`substr` counts characters) so a thousand
/// long notes never cross the socket in full just to draw a sidebar.
/// The window truncates further for display.
#[derive(Debug, PartialEq, Eq)]
pub struct NoteSummary {
    pub id: i64,
    pub title: String,
    pub created: String,
    pub snippet: String,
}

/// A whole note, body included.
///
/// The body was stored from the first commit and SEARCHED from the
/// first commit (`body LIKE ?1`) and never returned by anything. So
/// `search_notes` could find a note by a word in its body and then
/// hand back a title, and nothing — not the model, not a window —
/// could read what it had found. Notes was write-only for content
/// (#282 follow-up, found 2026-08-06 by building the window and
/// discovering there was nothing to put in it).
#[derive(Debug, PartialEq, Eq)]
pub struct Note {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub created: String,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS notes(
    id      INTEGER PRIMARY KEY,
    title   TEXT NOT NULL,
    body    TEXT NOT NULL,
    created TEXT NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0
)";

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open the database at `path` (created on first use) and migrate.
    /// The parent directory must already exist.
    pub fn open(path: &Path) -> rusqlite::Result<Store> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn })
    }

    /// Insert a note; returns its id. `created` is stamped by SQLite
    /// (UTC, RFC 3339) so this crate needs no clock dependency.
    pub fn create(&self, title: &str, body: &str) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO notes(title, body, created)
             VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![title, body],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Active (non-deleted) notes, oldest first.
    pub fn list(&self) -> rusqlite::Result<Vec<NoteSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, created, substr(body, 1, 200)
             FROM notes WHERE deleted = 0 ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(NoteSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                created: row.get(2)?,
                snippet: row.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// One note in full, or `None` if it does not exist or is deleted.
    ///
    /// `deleted = 0` deliberately: a soft-deleted note is gone as far
    /// as reading is concerned, and `restore_note` is the only way back.
    /// Returning the body of something the person deleted — to a model,
    /// on request — would make the delete a lie.
    pub fn read(&self, id: i64) -> rusqlite::Result<Option<Note>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, title, body, created FROM notes WHERE id = ?1 AND deleted = 0")?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(Note {
                id: row.get(0)?,
                title: row.get(1)?,
                body: row.get(2)?,
                created: row.get(3)?,
            })
        })?;
        rows.next().transpose()
    }

    /// Replace a note's title and body, returning what it held BEFORE.
    ///
    /// The previous value comes back so the caller can undo — the bus's
    /// undo journal compensates by calling `update_note` again with what
    /// this returned, the same shape `delete_note` ↔ `restore_note`
    /// already uses. An update with nothing to undo to would be the one
    /// write on this surface a person could not take back.
    ///
    /// `None` when no *active* note has that id: a soft-deleted note is
    /// not editable, for the same reason it is not readable.
    pub fn update(&self, id: i64, title: &str, body: &str) -> rusqlite::Result<Option<Note>> {
        let Some(before) = self.read(id)? else {
            return Ok(None);
        };
        self.conn.execute(
            "UPDATE notes SET title = ?2, body = ?3 WHERE id = ?1 AND deleted = 0",
            params![id, title, body],
        )?;
        Ok(Some(before))
    }

    /// Active notes whose title or body contains `query` as a literal
    /// substring, newest first (`created` desc, id as tiebreak), capped
    /// at `limit`. Matching is SQLite `LIKE`: case-insensitive for
    /// ASCII letters only — non-ASCII letters compare case-sensitively.
    /// `%`, `_`, and `\` in the query are escaped, so they match
    /// themselves, never as wildcards.
    pub fn search(&self, query: &str, limit: i64) -> rusqlite::Result<Vec<NoteSummary>> {
        let pattern = format!("%{}%", escape_like(query));
        let mut stmt = self.conn.prepare(
            "SELECT id, title, created, substr(body, 1, 200) FROM notes
             WHERE deleted = 0
               AND (title LIKE ?1 ESCAPE '\\' OR body LIKE ?1 ESCAPE '\\')
             ORDER BY created DESC, id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit], |row| {
            Ok(NoteSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                created: row.get(2)?,
                snippet: row.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// Soft-delete. `false` when no *active* note has that id (unknown
    /// or already deleted) — the caller turns that into a tool error.
    pub fn delete(&self, id: i64) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "UPDATE notes SET deleted = 1 WHERE id = ?1 AND deleted = 0",
            params![id],
        )?;
        Ok(n > 0)
    }

    /// Undo of [`Store::delete`]. `false` when no *deleted* note has
    /// that id (unknown or still active).
    pub fn restore(&self, id: i64) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "UPDATE notes SET deleted = 0 WHERE id = ?1 AND deleted = 1",
            params![id],
        )?;
        Ok(n > 0)
    }
}

/// Escape `LIKE` wildcards (`%`, `_`) and the escape character itself
/// so a user query only ever matches literally.
fn escape_like(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    for c in query.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("notes.db")).unwrap();
        (dir, store)
    }

    /// The gap this closes: a body could be stored and searched and
    /// never read back. `search` matching on body while nothing could
    /// return one is the exact shape of the defect.
    #[test]
    fn a_body_can_be_read_back_after_being_searched_for() {
        let (_dir, store) = fixture();
        let id = store.create("groceries", "oat milk and bread").unwrap();

        // Findable by a word that appears ONLY in the body...
        let hits = store.search("oat", 20).unwrap();
        assert_eq!(hits.len(), 1, "search matches the body");
        assert_eq!(hits[0].id, id);

        // ...and now readable, which it was not before.
        let note = store.read(id).unwrap().expect("the note exists");
        assert_eq!(note.title, "groceries");
        assert_eq!(note.body, "oat milk and bread");
        assert!(!note.created.is_empty());
    }

    /// A soft-deleted note is gone as far as reading is concerned.
    /// Returning its body on request would make the delete a lie.
    #[test]
    fn a_deleted_note_cannot_be_read_until_it_is_restored() {
        let (_dir, store) = fixture();
        let id = store.create("secret", "the body").unwrap();
        assert!(store.read(id).unwrap().is_some());

        assert!(store.delete(id).unwrap());
        assert!(
            store.read(id).unwrap().is_none(),
            "deleted stays unreadable"
        );

        assert!(store.restore(id).unwrap());
        assert_eq!(store.read(id).unwrap().unwrap().body, "the body");
    }

    /// An update returns the PREVIOUS value, which is what makes it
    /// undoable — the one write on this surface that would otherwise be
    /// impossible to take back.
    #[test]
    fn updating_returns_what_the_note_held_before() {
        let (_dir, store) = fixture();
        let id = store.create("draft", "first thoughts").unwrap();

        let before = store
            .update(id, "final", "considered thoughts")
            .unwrap()
            .unwrap();
        assert_eq!(before.title, "draft");
        assert_eq!(before.body, "first thoughts");

        let now = store.read(id).unwrap().unwrap();
        assert_eq!(now.title, "final");
        assert_eq!(now.body, "considered thoughts");

        // ...and undo is just the same call with what came back.
        store.update(id, &before.title, &before.body).unwrap();
        assert_eq!(store.read(id).unwrap().unwrap().body, "first thoughts");
    }

    #[test]
    fn a_deleted_note_cannot_be_edited() {
        let (_dir, store) = fixture();
        let id = store.create("gone", "body").unwrap();
        assert!(store.delete(id).unwrap());
        assert!(store.update(id, "back", "sneaky").unwrap().is_none());
        // ...and the write did not land underneath the refusal.
        assert!(store.restore(id).unwrap());
        assert_eq!(store.read(id).unwrap().unwrap().title, "gone");
    }

    #[test]
    fn updating_a_note_that_never_existed_changes_nothing() {
        let (_dir, store) = fixture();
        assert!(store.update(9999, "x", "y").unwrap().is_none());
    }

    #[test]
    fn reading_a_note_that_never_existed_is_none_not_an_error() {
        let (_dir, store) = fixture();
        assert!(store.read(9999).unwrap().is_none());
    }

    #[test]
    fn create_then_list_returns_the_summary() {
        let (_dir, store) = fixture();
        let id = store.create("first", "hello").unwrap();
        assert!(id > 0);
        let second = store.create("second", "").unwrap();
        assert!(second > id, "ids are monotonic");

        let notes = store.list().unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].id, id);
        assert_eq!(notes[0].title, "first");
        assert!(
            notes[0].created.ends_with('Z') && notes[0].created.contains('T'),
            "created is an RFC 3339 UTC stamp: {:?}",
            notes[0].created
        );
    }

    /// A list row carries the opening of its body so a sidebar can show
    /// a preview without reading every note in full — and a long body
    /// is cut at 200 *characters* (SQLite `substr` semantics), so a
    /// multibyte body cannot come back torn mid-codepoint.
    #[test]
    fn a_summary_carries_a_snippet_cut_by_characters_not_bytes() {
        let (_dir, store) = fixture();
        store.create("short", "oat milk").unwrap();
        store.create("long", &"š".repeat(500)).unwrap();

        let notes = store.list().unwrap();
        assert_eq!(notes[0].snippet, "oat milk");
        assert_eq!(
            notes[1].snippet.chars().count(),
            200,
            "cut at 200 characters even when each is 2 bytes"
        );
        assert!(
            notes[1].snippet.chars().all(|c| c == 'š'),
            "no torn codepoint"
        );

        // Search summaries carry the same snippet.
        let hits = store.search("oat", 20).unwrap();
        assert_eq!(hits[0].snippet, "oat milk");
    }

    #[test]
    fn delete_hides_from_list_and_restore_brings_it_back() {
        let (_dir, store) = fixture();
        let id = store.create("ephemeral", "bye").unwrap();

        assert!(store.delete(id).unwrap());
        assert!(store.list().unwrap().is_empty());

        assert!(store.restore(id).unwrap());
        let notes = store.list().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, id);
    }

    #[test]
    fn delete_and_restore_are_strict_about_state() {
        let (_dir, store) = fixture();
        let id = store.create("note", "").unwrap();

        assert!(!store.delete(999).unwrap(), "unknown id deletes nothing");
        assert!(store.delete(id).unwrap());
        assert!(!store.delete(id).unwrap(), "already deleted");

        assert!(!store.restore(999).unwrap(), "unknown id restores nothing");
        assert!(store.restore(id).unwrap());
        assert!(!store.restore(id).unwrap(), "already active");
    }

    #[test]
    fn search_matches_title_and_body_newest_first() {
        let (_dir, store) = fixture();
        let a = store.create("milk run", "eggs and bread").unwrap();
        let b = store.create("meeting", "budget for milk").unwrap();
        store.create("unrelated", "nothing here").unwrap();

        let hits = store.search("milk", 20).unwrap();
        assert_eq!(
            hits.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![b, a],
            "title and body both match, newest first"
        );
    }

    #[test]
    fn search_is_case_insensitive_for_ascii_only() {
        // SQLite LIKE folds case for the 26 ASCII letters only —
        // non-ASCII letters compare case-sensitively (SQLite's
        // documented default). Documented here, not worked around.
        let (_dir, store) = fixture();
        store.create("Shopping List", "MILK").unwrap();
        assert_eq!(
            store.search("shopping", 20).unwrap().len(),
            1,
            "title, folded"
        );
        assert_eq!(store.search("milk", 20).unwrap().len(), 1, "body, folded");

        store.create("Škoda", "").unwrap();
        assert_eq!(
            store.search("Škoda", 20).unwrap().len(),
            1,
            "exact non-ASCII"
        );
        assert!(
            store.search("škoda", 20).unwrap().is_empty(),
            "non-ASCII case is not folded"
        );
    }

    #[test]
    fn search_excludes_soft_deleted_notes() {
        let (_dir, store) = fixture();
        let keep = store.create("keep milk", "").unwrap();
        let gone = store.create("gone milk", "").unwrap();
        assert!(store.delete(gone).unwrap());

        let hits = store.search("milk", 20).unwrap();
        assert_eq!(hits.iter().map(|n| n.id).collect::<Vec<_>>(), vec![keep]);

        assert!(store.restore(gone).unwrap());
        assert_eq!(store.search("milk", 20).unwrap().len(), 2);
    }

    #[test]
    fn search_honors_the_limit_keeping_the_newest() {
        let (_dir, store) = fixture();
        store.create("milk 1", "").unwrap();
        let b = store.create("milk 2", "").unwrap();
        let c = store.create("milk 3", "").unwrap();

        let hits = store.search("milk", 2).unwrap();
        assert_eq!(
            hits.iter().map(|n| n.id).collect::<Vec<_>>(),
            vec![c, b],
            "limit trims the oldest matches"
        );
    }

    #[test]
    fn search_treats_like_wildcards_literally() {
        let (_dir, store) = fixture();
        store.create("progress", "50% done").unwrap();
        store.create("naming", "snake_case wins").unwrap();
        store.create("paths", r"C:\temp").unwrap();
        store.create("plain", "abc").unwrap();

        let titles = |q: &str| {
            store
                .search(q, 20)
                .unwrap()
                .into_iter()
                .map(|n| n.title)
                .collect::<Vec<_>>()
        };
        assert_eq!(titles("%"), vec!["progress"], "% is literal, not match-all");
        assert_eq!(titles("50% d"), vec!["progress"]);
        assert_eq!(titles("_"), vec!["naming"], "_ is literal, not any-char");
        assert_eq!(titles(r"\"), vec!["paths"], "backslash is literal too");
        assert!(
            titles("50_ done").is_empty(),
            "_ does not act as an any-character wildcard"
        );
    }

    #[test]
    fn reopening_the_db_keeps_the_notes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.db");
        let id = Store::open(&path).unwrap().create("durable", "x").unwrap();
        let notes = Store::open(&path).unwrap().list().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, id);
    }
}
