CREATE TABLE thread_skills (
    thread_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    path TEXT NOT NULL,
    kind TEXT NOT NULL,
    PRIMARY KEY(thread_id, position),
    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
);

CREATE INDEX idx_thread_skills_thread ON thread_skills(thread_id);
