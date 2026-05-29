-- SundayStudio project schema (Phase 2.1).
--
-- The "tape model" every DAW uses: a Take is raw recorded audio (one WAV per
-- track on disk); a Region references a time range within a take and places it
-- on the timeline. Originals are never mutated — all editing is non-destructive
-- region math. The .sqlite file stays small (< 5 MB) because it holds only this
-- metadata, never audio (addressing GarageBand's "huge files" complaint).
--
-- Conventions: ids are TEXT (UUID v7, time-ordered). Time/position/duration are
-- REAL milliseconds. Booleans are INTEGER 0/1. One project per database file.

PRAGMA foreign_keys = ON;

CREATE TABLE project (
    id            TEXT PRIMARY KEY NOT NULL,
    name          TEXT NOT NULL,
    sample_rate   INTEGER NOT NULL DEFAULT 48000,
    channel_count INTEGER NOT NULL DEFAULT 2,
    created_at    REAL NOT NULL
);

CREATE TABLE track (
    id                TEXT PRIMARY KEY NOT NULL,
    project_id        TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    color             TEXT NOT NULL DEFAULT '#D4A73A',
    input_assignment  INTEGER,            -- interface channel, NULL = unassigned
    output_assignment INTEGER,
    gain_db           REAL NOT NULL DEFAULT 0,
    pan               REAL NOT NULL DEFAULT 0,   -- -1 left .. +1 right
    mute              INTEGER NOT NULL DEFAULT 0,
    solo              INTEGER NOT NULL DEFAULT 0,
    armed             INTEGER NOT NULL DEFAULT 0,
    position          INTEGER NOT NULL DEFAULT 0  -- order in the track list
);
CREATE INDEX idx_track_project ON track(project_id, position);

CREATE TABLE take (
    id            TEXT PRIMARY KEY NOT NULL,
    project_id    TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    started_at    REAL NOT NULL,
    duration_ms   REAL NOT NULL DEFAULT 0,
    -- JSON array of track ids captured in this take.
    source_tracks TEXT NOT NULL DEFAULT '[]'
);
CREATE INDEX idx_take_project ON take(project_id, started_at);

CREATE TABLE region (
    id                     TEXT PRIMARY KEY NOT NULL,
    take_id                TEXT NOT NULL REFERENCES take(id) ON DELETE CASCADE,
    source_track_id        TEXT NOT NULL,
    target_track_id        TEXT NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    start_in_take_ms       REAL NOT NULL DEFAULT 0,
    end_in_take_ms         REAL NOT NULL DEFAULT 0,
    position_in_timeline_ms REAL NOT NULL DEFAULT 0,
    fade_in_ms             REAL NOT NULL DEFAULT 5,
    fade_out_ms            REAL NOT NULL DEFAULT 5,
    gain_adjust_db         REAL NOT NULL DEFAULT 0
);
CREATE INDEX idx_region_target ON region(target_track_id, position_in_timeline_ms);

CREATE TABLE marker (
    id          TEXT PRIMARY KEY NOT NULL,
    project_id  TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    position_ms REAL NOT NULL,
    label       TEXT NOT NULL DEFAULT '',
    color       TEXT NOT NULL DEFAULT '#D4A73A'
);
CREATE INDEX idx_marker_project ON marker(project_id, position_ms);

-- Forward-compat tables (full CRUD lands in their phases): per-track effect
-- chain (Phase 4), parameter automation (Phase 4), and jingles (Phase 6).
CREATE TABLE effect_instance (
    id         TEXT PRIMARY KEY NOT NULL,
    track_id   TEXT NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    effect_type TEXT NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0,
    bypassed   INTEGER NOT NULL DEFAULT 0,
    params     TEXT NOT NULL DEFAULT '{}'   -- JSON
);
CREATE INDEX idx_effect_track ON effect_instance(track_id, position);

CREATE TABLE automation_lane (
    id        TEXT PRIMARY KEY NOT NULL,
    track_id  TEXT NOT NULL REFERENCES track(id) ON DELETE CASCADE,
    parameter TEXT NOT NULL,
    points    TEXT NOT NULL DEFAULT '[]'    -- JSON array of {ms, value}
);

CREATE TABLE jingle_asset (
    id               TEXT PRIMARY KEY NOT NULL,
    project_id       TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    name             TEXT NOT NULL,
    audio_file       TEXT NOT NULL,
    used_at_positions TEXT NOT NULL DEFAULT '[]'  -- JSON
);
