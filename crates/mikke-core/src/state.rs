//! État persistant de l'indexation incrémentale (SQLite, WAL).
//!
//! `files` mémorise mtime + taille + hash blake3 de chaque fichier indexé ;
//! `chunks` mémorise les embeddings (BLOB f32 LE) pour reconstruire l'index
//! HNSW sans jamais ré-embedder les fichiers inchangés.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

pub const DB_FILE: &str = "state.db";

pub struct State {
    conn: Connection,
}

pub struct FileRecord {
    pub mtime_ns: i64,
    pub size: i64,
    pub hash: [u8; 32],
}

impl State {
    pub fn open(index_dir: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(index_dir.join(DB_FILE))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS files (
                path     TEXT PRIMARY KEY,
                mtime_ns INTEGER NOT NULL,
                size     INTEGER NOT NULL,
                hash     BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS chunks (
                id       INTEGER PRIMARY KEY AUTOINCREMENT,
                path     TEXT NOT NULL,
                chunk_ix INTEGER NOT NULL,
                vector   BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS chunks_path ON chunks(path);",
        )?;
        Ok(Self { conn })
    }

    pub fn file(&self, path: &str) -> rusqlite::Result<Option<FileRecord>> {
        self.conn
            .query_row(
                "SELECT mtime_ns, size, hash FROM files WHERE path = ?1",
                params![path],
                |row| {
                    let hash: Vec<u8> = row.get(2)?;
                    Ok(FileRecord {
                        mtime_ns: row.get(0)?,
                        size: row.get(1)?,
                        hash: hash.try_into().unwrap_or([0; 32]),
                    })
                },
            )
            .optional()
    }

    pub fn upsert_file(&self, path: &str, rec: &FileRecord) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO files (path, mtime_ns, size, hash) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET mtime_ns = ?2, size = ?3, hash = ?4",
            params![path, rec.mtime_ns, rec.size, rec.hash.as_slice()],
        )?;
        Ok(())
    }

    /// Supprime les chunks d'un fichier (avant réémission ou disparition).
    pub fn delete_chunks(&self, path: &str) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM chunks WHERE path = ?1", params![path])?;
        Ok(())
    }

    pub fn delete_file(&self, path: &str) -> rusqlite::Result<()> {
        self.delete_chunks(path)?;
        self.conn
            .execute("DELETE FROM files WHERE path = ?1", params![path])?;
        Ok(())
    }

    /// Insère un chunk et retourne son id (stable pour tantivy et HNSW).
    pub fn insert_chunk(&self, path: &str, chunk_ix: u64, vector: &[f32]) -> rusqlite::Result<u64> {
        let blob: Vec<u8> = vector.iter().flat_map(|f| f.to_le_bytes()).collect();
        self.conn.execute(
            "INSERT INTO chunks (path, chunk_ix, vector) VALUES (?1, ?2, ?3)",
            params![path, chunk_ix as i64, blob],
        )?;
        Ok(self.conn.last_insert_rowid() as u64)
    }

    pub fn all_paths(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM files")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    /// Tous les vecteurs non vides, pour reconstruire l'index HNSW.
    pub fn all_vectors(&self) -> rusqlite::Result<Vec<(u64, Vec<f32>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, vector FROM chunks WHERE length(vector) > 0")?;
        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            let vector = blob
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().expect("chunks_exact(4)")))
                .collect();
            Ok((id as u64, vector))
        })?;
        rows.collect()
    }

    pub fn chunk_count(&self) -> rusqlite::Result<u64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
            .map(|n: i64| n as u64)
    }

    pub fn begin(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch("BEGIN")
    }

    pub fn commit(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch("COMMIT")
    }
}
