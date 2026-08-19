import { Database } from "bun:sqlite";
import { join } from "node:path";

const dbPath = join(process.cwd(), "bot_data.sqlite");
export const db = new Database(dbPath, { create: true });

// เปิดใช้งาน WAL mode เพื่อความเร็วและปลอดภัย
db.run("PRAGMA journal_mode = WAL;");

// สร้างตารางเบื้องต้น
db.run(`
  CREATE TABLE IF NOT EXISTS song_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    guild_id TEXT NOT NULL,
    title TEXT NOT NULL,
    author TEXT NOT NULL,
    uri TEXT NOT NULL,
    requested_by TEXT NOT NULL,
    played_at DATETIME DEFAULT CURRENT_TIMESTAMP
  );
`);

db.run(`
  CREATE TABLE IF NOT EXISTS guild_settings (
    guild_id TEXT PRIMARY KEY,
    prefix TEXT DEFAULT '\\',
    default_volume INTEGER DEFAULT 100,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
  );
`);

export function logPlayedTrack(guildId: string, title: string, author: string, uri: string, requestedBy: string) {
  const stmt = db.prepare(`
    INSERT INTO song_history (guild_id, title, author, uri, requested_by)
    VALUES (?, ?, ?, ?, ?)
  `);
  stmt.run(guildId, title, author, uri, requestedBy);
}

export function getRecentTracks(guildId: string, limit = 10) {
  const stmt = db.prepare(`
    SELECT title, author, uri, requested_by, played_at
    FROM song_history
    WHERE guild_id = ?
    ORDER BY id DESC
    LIMIT ?
  `);
  return stmt.all(guildId, limit);
}
