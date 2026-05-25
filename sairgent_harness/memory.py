import sqlite3
from datetime import datetime, timezone
from typing import List, Dict, Any, Optional

class AgentMemory:
    """
    Sovereign Memory Interface.
    Each agent has a dedicated SQLite database managed by the Rust plane.
    """
    def __init__(self, db_path: str):
        self.conn = sqlite3.connect(db_path)
        self.conn.row_factory = sqlite3.Row
        self.conn.execute("PRAGMA busy_timeout = 5000")
        self._init_schema()

    def _init_schema(self):
        cursor = self.conn.cursor()
        cursor.execute('''
            CREATE TABLE IF NOT EXISTS interactions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                swo_id INTEGER,
                mode TEXT NOT NULL DEFAULT 'legacy',
                run_id TEXT,
                interaction_kind TEXT NOT NULL DEFAULT 'message'
            )
        ''')
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_interactions_swo_id ON interactions(swo_id)')
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_interactions_mode ON interactions(mode)')
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_interactions_run_id ON interactions(run_id)')
        try:
            cursor.execute('ALTER TABLE interactions ADD COLUMN swo_id INTEGER')
        except sqlite3.OperationalError:
            pass
        try:
            cursor.execute("ALTER TABLE interactions ADD COLUMN mode TEXT NOT NULL DEFAULT 'legacy'")
        except sqlite3.OperationalError:
            pass
        try:
            cursor.execute("ALTER TABLE interactions ADD COLUMN run_id TEXT")
        except sqlite3.OperationalError:
            pass
        try:
            cursor.execute("ALTER TABLE interactions ADD COLUMN interaction_kind TEXT NOT NULL DEFAULT 'message'")
        except sqlite3.OperationalError:
            pass
        cursor.execute('''
            CREATE TABLE IF NOT EXISTS decision_log (
                entry_id TEXT PRIMARY KEY,
                mode TEXT NOT NULL,
                summary TEXT NOT NULL,
                rationale TEXT NOT NULL,
                outcome TEXT NOT NULL,
                confidence REAL,
                self_note TEXT,
                linked_swo_id INTEGER,
                linked_run_id TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        ''')
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_decision_log_created_at ON decision_log(created_at DESC)')
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_decision_log_mode ON decision_log(mode)')
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_decision_log_linked_swo_id ON decision_log(linked_swo_id)')
        self.conn.commit()

    def append_interaction(
        self,
        role: str,
        content: str,
        swo_id: Optional[int] = None,
        mode: str = "legacy",
        run_id: Optional[str] = None,
        interaction_kind: str = "message",
    ):
        cursor = self.conn.cursor()
        cursor.execute(
            '''
            INSERT INTO interactions (role, content, swo_id, mode, run_id, interaction_kind)
            VALUES (?, ?, ?, ?, ?, ?)
            ''',
            (role, content, swo_id, mode, run_id, interaction_kind)
        )
        self.conn.commit()

    def get_history(self, limit: int = 50, mode: Optional[str] = None, exclude_swo_ids: Optional[List[int]] = None) -> List[Dict[str, Any]]:
        cursor = self.conn.cursor()
        if exclude_swo_ids:
            placeholders = ",".join("?" for _ in exclude_swo_ids)
            if mode is None:
                cursor.execute(
                    f'SELECT role, content FROM interactions WHERE (swo_id IS NULL OR swo_id NOT IN ({placeholders})) ORDER BY id ASC LIMIT ?',
                    (*exclude_swo_ids, limit)
                )
            else:
                cursor.execute(
                    f'SELECT role, content FROM interactions WHERE mode = ? AND (swo_id IS NULL OR swo_id NOT IN ({placeholders})) ORDER BY id ASC LIMIT ?',
                    (mode, *exclude_swo_ids, limit)
                )
        elif mode is None:
            cursor.execute(
                'SELECT role, content FROM interactions ORDER BY id ASC LIMIT ?',
                (limit,)
            )
        else:
            cursor.execute(
                'SELECT role, content FROM interactions WHERE mode = ? ORDER BY id ASC LIMIT ?',
                (mode, limit)
            )
        return [{"role": row[0], "content": row[1]} for row in cursor.fetchall()]

    def append_decision_log_entry(
        self,
        entry_id: str,
        mode: str,
        summary: str,
        rationale: str,
        outcome: str,
        confidence: Optional[float] = None,
        self_note: Optional[str] = None,
        linked_swo_id: Optional[int] = None,
        linked_run_id: Optional[str] = None,
        created_at: Optional[str] = None,
    ) -> Dict[str, Any]:
        timestamp = created_at or datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S")
        cursor = self.conn.cursor()
        cursor.execute(
            '''
            INSERT INTO decision_log (
                entry_id,
                mode,
                summary,
                rationale,
                outcome,
                confidence,
                self_note,
                linked_swo_id,
                linked_run_id,
                created_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ''',
            (
                entry_id,
                mode,
                summary,
                rationale,
                outcome,
                confidence,
                self_note,
                linked_swo_id,
                linked_run_id,
                timestamp,
            )
        )
        self.conn.commit()
        return {
            "entry_id": entry_id,
            "mode": mode,
            "summary": summary,
            "rationale": rationale,
            "outcome": outcome,
            "confidence": confidence,
            "self_note": self_note,
            "linked_swo_id": linked_swo_id,
            "linked_run_id": linked_run_id,
            "created_at": timestamp,
        }

    def list_decision_log(self, limit: int = 50) -> List[Dict[str, Any]]:
        cursor = self.conn.cursor()
        cursor.execute(
            '''
            SELECT
                entry_id,
                mode,
                summary,
                rationale,
                outcome,
                confidence,
                self_note,
                linked_swo_id,
                linked_run_id,
                created_at
            FROM decision_log
            ORDER BY created_at DESC, entry_id DESC
            LIMIT ?
            ''',
            (max(1, limit),)
        )
        return [dict(row) for row in cursor.fetchall()]

    def prune_decision_log(self, max_entries: int) -> int:
        cursor = self.conn.cursor()
        if max_entries <= 0:
            cursor.execute('DELETE FROM decision_log')
        else:
            cursor.execute(
                '''
                DELETE FROM decision_log
                WHERE entry_id IN (
                    SELECT entry_id
                    FROM decision_log
                    ORDER BY created_at DESC, entry_id DESC
                    LIMIT -1 OFFSET ?
                )
                ''',
                (max_entries,)
            )
        self.conn.commit()
        return cursor.rowcount

    def format_decision_log_context(self, limit: int = 5) -> str:
        rows = self.list_decision_log(limit)
        if not rows:
            return "No prior decision log entries recorded."
        lines = []
        for row in rows:
            detail = f"{row['created_at']} [{row['mode']}] {row['summary']} ({row['outcome']})"
            if row.get("self_note"):
                detail += f" | self_note: {row['self_note']}"
            if row.get("linked_swo_id") is not None:
                detail += f" | swo: {row['linked_swo_id']}"
            lines.append(f"- {detail}")
        return "\n".join(lines)

    def close(self):
        self.conn.close()
