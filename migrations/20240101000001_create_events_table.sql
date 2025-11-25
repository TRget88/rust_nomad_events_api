ALTER TABLE events ADD COLUMN event_type TEXT;
ALTER TABLE events ADD COLUMN latitude REAL;
ALTER TABLE events ADD COLUMN longitude REAL;
ALTER TABLE events ADD COLUMN start_date TEXT;
ALTER TABLE events ADD COLUMN end_date TEXT;
ALTER TABLE events ADD COLUMN camping_allowed BOOLEAN DEFAULT 0;

CREATE INDEX idx_events_location ON events(latitude, longitude);
CREATE INDEX idx_events_type ON events(event_type);
CREATE INDEX idx_events_dates ON events(start_date, end_date);