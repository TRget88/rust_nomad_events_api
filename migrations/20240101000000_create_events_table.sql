-- Create events table
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    website TEXT,
    event_data TEXT NOT NULL,  -- JSON blob containing full event details
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Create index on name for faster searches
CREATE INDEX IF NOT EXISTS idx_events_name ON events(name);

-- Trigger to update the updated_at timestamp
CREATE TRIGGER IF NOT EXISTS update_events_timestamp 
AFTER UPDATE ON events
BEGIN
    UPDATE events SET updated_at = CURRENT_TIMESTAMP WHERE id = NEW.id;
END;