//! Memory — per-scope durable notes, the "second brain" pillar. Notes are
//! remembered under a scope (an identity, a project, a room); each turn
//! the caller asks for a bounded [`Memory::digest`] of the scope and
//! injects it into the system prompt, and can [`Memory::recall`] on
//! demand. Search is FTS5 (as contextd uses); stores built without FTS5
//! degrade to LIKE matching instead of failing.

use crate::{Error, now_millis};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

/// How many of a scope's newest notes compete for digest space. Recency
/// beyond this rarely matters for a prompt, and bounding it keeps the
/// ranking pass O(64) regardless of store size.
const DIGEST_CANDIDATES: i64 = 64;

/// One remembered note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub id: i64,
    /// Unix milliseconds.
    pub ts: i64,
    pub scope: String,
    pub text: String,
    pub tags: Vec<String>,
    /// How often [`Memory::recall`] has surfaced this note — the
    /// reinforcement signal [`Memory::digest`] ranks by.
    pub recalls: i64,
}

/// A scoped memory store at a caller-supplied SQLite path.
pub struct Memory {
    conn: Mutex<Connection>,
    fts: bool,
}

impl Memory {
    /// Open (creating if needed) the store at `path`. FTS5 is probed, not
    /// assumed: bundled rusqlite compiles SQLite with it, but a build
    /// without it falls back to LIKE search.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        if let Some(dir) = path.as_ref().parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path.as_ref())?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS notes (
                id      INTEGER PRIMARY KEY AUTOINCREMENT,
                ts      INTEGER NOT NULL,
                scope   TEXT NOT NULL,
                text    TEXT NOT NULL,
                tags    TEXT NOT NULL DEFAULT '',
                recalls INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS notes_scope ON notes(scope, id);",
        )?;
        let fts = conn
            .execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
                    text, tags, scope UNINDEXED, note_id UNINDEXED
                )",
            )
            .is_ok();
        Ok(Self {
            conn: Mutex::new(conn),
            fts,
        })
    }

    /// Whether this store has FTS5 (false = LIKE fallback is in use).
    pub fn has_fts(&self) -> bool {
        self.fts
    }

    /// Remember `text` under `scope`, with searchable `tags`.
    pub fn remember(&self, scope: &str, text: &str, tags: &[&str]) -> Result<i64, Error> {
        let tags_joined = tags.join(",");
        let mut conn = self.conn.lock().expect("memory lock");
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO notes (ts, scope, text, tags) VALUES (?1, ?2, ?3, ?4)",
            params![now_millis(), scope, text, tags_joined],
        )?;
        let id = tx.last_insert_rowid();
        if self.fts {
            tx.execute(
                "INSERT INTO notes_fts (text, tags, scope, note_id) VALUES (?1, ?2, ?3, ?4)",
                params![text, tags_joined, scope, id],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// The notes in `scope` matching `query`, best first. Every surfaced
    /// note's `recalls` counter is bumped — recall is reinforcement, and
    /// the digest ranks by it. A query with no searchable tokens matches
    /// nothing (rather than everything).
    pub fn recall(&self, scope: &str, query: &str, limit: usize) -> Result<Vec<Note>, Error> {
        let conn = self.conn.lock().expect("memory lock");
        let notes = if self.fts {
            let q = fts_query(query);
            if q.is_empty() {
                Vec::new()
            } else {
                let mut stmt = conn.prepare(
                    "SELECT n.id, n.ts, n.scope, n.text, n.tags, n.recalls
                     FROM notes_fts JOIN notes n ON n.id = notes_fts.note_id
                     WHERE notes_fts MATCH ?1 AND notes_fts.scope = ?2
                     ORDER BY bm25(notes_fts) LIMIT ?3",
                )?;
                let rows = stmt.query_map(params![q, scope, limit as i64], map_note)?;
                rows.collect::<Result<_, _>>()?
            }
        } else {
            let pattern = like_pattern(query);
            let mut stmt = conn.prepare(
                "SELECT id, ts, scope, text, tags, recalls FROM notes
                 WHERE scope = ?1 AND (text LIKE ?2 ESCAPE '\\' OR tags LIKE ?2 ESCAPE '\\')
                 ORDER BY id DESC LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![scope, pattern, limit as i64], map_note)?;
            rows.collect::<Result<_, _>>()?
        };
        for n in &notes {
            conn.execute(
                "UPDATE notes SET recalls = recalls + 1 WHERE id = ?1",
                [n.id],
            )?;
        }
        Ok(notes)
    }

    /// The bounded digest a caller injects into the system prompt each
    /// turn: one `- <text>` line per note, most-relevant first, never
    /// exceeding `budget_chars`. Relevance blends reinforcement and
    /// recency (`recalls * 4 + position-from-oldest-candidate`), so notes
    /// the model keeps needing survive, and fresh notes always get a
    /// hearing. If the single best note doesn't fit whole it is
    /// truncated with an ellipsis; the cap is hard.
    pub fn digest(&self, scope: &str, budget_chars: usize) -> Result<String, Error> {
        if budget_chars == 0 {
            return Ok(String::new());
        }
        let candidates = self.candidates(scope)?;
        if candidates.is_empty() {
            return Ok(String::new());
        }
        let n = candidates.len() as i64;
        let mut scored: Vec<(i64, &Note)> = candidates
            .iter()
            .enumerate()
            .map(|(i, note)| (note.recalls * 4 + (n - i as i64), note))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.id.cmp(&a.1.id)));

        let mut out = String::new();
        for (_, note) in &scored {
            let line = format!("- {}", one_line(&note.text));
            let extra = line.chars().count() + usize::from(!out.is_empty());
            if out.chars().count() + extra <= budget_chars {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&line);
            }
        }
        if out.is_empty() {
            // Even the best note doesn't fit whole: truncate it to the cap.
            let text = one_line(&scored[0].1.text);
            let room = budget_chars.saturating_sub(3); // "- " + '…'
            let truncated: String = text.chars().take(room).collect();
            out = format!("- {truncated}…")
                .chars()
                .take(budget_chars)
                .collect();
        }
        Ok(out)
    }

    /// The scope's newest notes, newest first — the digest candidate pool.
    fn candidates(&self, scope: &str) -> Result<Vec<Note>, Error> {
        let conn = self.conn.lock().expect("memory lock");
        let mut stmt = conn.prepare(
            "SELECT id, ts, scope, text, tags, recalls FROM notes
             WHERE scope = ?1 ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![scope, DIGEST_CANDIDATES], map_note)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

fn map_note(r: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    let tags: String = r.get(4)?;
    Ok(Note {
        id: r.get(0)?,
        ts: r.get(1)?,
        scope: r.get(2)?,
        text: r.get(3)?,
        tags: if tags.is_empty() {
            Vec::new()
        } else {
            tags.split(',').map(str::to_string).collect()
        },
        recalls: r.get(5)?,
    })
}

/// Build a safe FTS5 query from free text: keep word-ish tokens, quote
/// each (so FTS operators in user text are inert), OR them together.
fn fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|t| {
            t.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
        })
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// `%query%` with LIKE's wildcards and the escape char itself escaped.
fn like_pattern(query: &str) -> String {
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// Digest lines are single-line; collapse embedded newlines.
fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_memory() -> (tempfile::TempDir, Memory) {
        let dir = tempfile::tempdir().unwrap();
        let mem = Memory::open(dir.path().join("memory.db")).unwrap();
        (dir, mem)
    }

    #[test]
    fn remember_and_recall_with_scope_isolation() {
        let (_dir, mem) = test_memory();
        mem.remember("user", "prefers dark theme in demos", &["ui"])
            .unwrap();
        mem.remember("user", "deploy target is the nuc box", &["infra"])
            .unwrap();
        mem.remember("work", "standup moved to 9:30", &[]).unwrap();

        let hits = mem.recall("user", "dark theme", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text, "prefers dark theme in demos");
        assert_eq!(hits[0].tags, vec!["ui".to_string()]);

        // Scopes don't leak into each other; tags are searchable too.
        assert!(mem.recall("user", "standup", 10).unwrap().is_empty());
        assert_eq!(mem.recall("work", "standup", 10).unwrap().len(), 1);
        assert_eq!(
            mem.recall("user", "infra", 10).unwrap()[0].text,
            "deploy target is the nuc box"
        );

        // Recall reinforces: the surfaced note's counter went up.
        let again = mem.recall("user", "dark", 10).unwrap();
        assert_eq!(again[0].recalls, 1);

        // FTS-operator-looking input is inert, and tokenless queries match nothing.
        assert!(mem.recall("user", "\"dark\" OR *", 10).is_ok());
        assert!(mem.recall("user", "!!!", 10).unwrap().is_empty());
    }

    #[test]
    fn recall_falls_back_to_like() {
        let (_dir, mut mem) = test_memory();
        mem.remember("user", "prefers dark theme", &["ui"]).unwrap();
        mem.fts = false; // simulate a SQLite build without FTS5
        assert!(!mem.has_fts());
        let hits = mem.recall("user", "dark", 10).unwrap();
        assert_eq!(hits.len(), 1);
        // LIKE wildcards in the query are literal, not magic.
        assert!(mem.recall("user", "100%", 10).unwrap().is_empty());
    }

    #[test]
    fn digest_respects_a_hard_budget() {
        let (_dir, mem) = test_memory();
        for i in 0..6 {
            mem.remember("user", &format!("note number {i} with some body text"), &[])
                .unwrap();
        }
        // Each line is ~35 chars: 80 fits two but not three.
        let digest = mem.digest("user", 80).unwrap();
        assert!(digest.chars().count() <= 80, "hard cap: {digest:?}");
        assert_eq!(digest.lines().count(), 2, "packs several notes: {digest:?}");
        assert!(digest.lines().all(|l| l.starts_with("- ")));

        // A single oversized note is truncated with an ellipsis, still capped.
        let tiny = mem.digest("user", 12).unwrap();
        assert!(tiny.chars().count() <= 12, "tiny cap: {tiny:?}");
        assert!(tiny.ends_with('…'));

        assert_eq!(mem.digest("user", 0).unwrap(), "");
        assert_eq!(mem.digest("nobody", 100).unwrap(), "");
    }

    #[test]
    fn digest_prefers_recalled_then_recent() {
        let (_dir, mem) = test_memory();
        mem.remember("user", "old but often needed", &[]).unwrap();
        for i in 0..5 {
            mem.remember("user", &format!("fresher note {i}"), &[])
                .unwrap();
        }
        // Reinforce the oldest note so it outranks fresher ones.
        for _ in 0..3 {
            mem.recall("user", "often needed", 10).unwrap();
        }
        // Budget that fits roughly one line: the reinforced note wins it.
        let digest = mem.digest("user", 25).unwrap();
        assert!(
            digest.contains("old but often needed"),
            "digest: {digest:?}"
        );

        // Without reinforcement the newest notes fill the digest.
        let (_dir2, fresh) = test_memory();
        fresh.remember("user", "first note ever", &[]).unwrap();
        fresh.remember("user", "second note", &[]).unwrap();
        let d = fresh.digest("user", 100).unwrap();
        let first_pos = d.find("first note ever").unwrap();
        let second_pos = d.find("second note").unwrap();
        assert!(second_pos < first_pos, "newest first: {d:?}");
    }
}
