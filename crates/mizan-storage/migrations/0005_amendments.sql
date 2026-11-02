-- Imported activities amended by hand keep the record as first imported here.
-- The activity row holds the amended record; reverting copies this back.

CREATE TABLE activity_amendments (
    activity_id   INTEGER PRIMARY KEY REFERENCES activities(id),
    original_json TEXT NOT NULL,
    amended_at    TEXT NOT NULL
);
