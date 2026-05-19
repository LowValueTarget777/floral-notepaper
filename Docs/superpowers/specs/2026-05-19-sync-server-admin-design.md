# Floral Sync Server Web Admin Design

## Background

`sync-server/` currently provides a lightweight single-user sync service for Floral Notepaper. The existing desktop client already depends on three stable endpoints:

- `GET /health`
- `GET /v1/changes?since=<revision>`
- `POST /v1/push`

Those endpoints are already integrated into the client and must remain wire-compatible. The new work adds an embedded web-based management console without changing the client protocol.

The server must continue to be easy to self-host and easy to extract into its own repository later. Deployment targets are:

- Windows `x86_64-pc-windows-msvc` executable
- Linux `x86_64-unknown-linux-gnu` binary
- Linux `x86_64-unknown-linux-musl` binary

## Goals

1. Keep the current sync API stable so the desktop client does not need protocol changes.
2. Add a modern web admin console that runs from the same Rust server binary.
3. Separate sync traffic and admin traffic onto different listen addresses and ports.
4. Use a dedicated admin password login instead of reusing the sync token.
5. Let the admin console view synchronized notes as a read-only backup surface.
6. Let the admin console manage operational settings such as listen addresses, token rotation, password changes, backup export, and log viewing.
7. Produce release artifacts for Windows, Linux GNU, and Linux musl.

## Non-Goals

1. No changes to the desktop client's sync request or response schema.
2. No multi-user support, sharing, or collaborative editing.
3. No note editing from the web console.
4. No note delete, restore, or merge actions from the web console.
5. No browser-based note conflict resolution workflow.
6. No dependency on a separate Node.js server at runtime.

## Selected Approach

The server will stay as one Rust application with two runtime surfaces:

- a sync API listener for the existing client
- an admin API plus embedded web UI listener for browser access

The management UI will be built in `sync-server/admin-ui/` with React and Vite, then embedded into the Rust binary at build time as static assets. At runtime, the user still launches a single server executable.

This approach keeps deployment simple while allowing the admin console to be modern, responsive, and easy to maintain.

## High-Level Architecture

### Runtime topology

One process hosts two independent routers:

1. **Sync router**
   - Handles `/health`, `/v1/changes`, `/v1/push`
   - Authenticates with `Authorization: Bearer <sync_token>`
   - Uses the existing sync protocol

2. **Admin router**
   - Handles `/login`, `/logout`, `/admin`, and `/admin/api/*`
   - Authenticates with password-backed session cookies
   - Serves embedded static assets for the management UI

Both routers share the same storage layer and configuration layer, but they do not share authentication or route namespaces.

### Default listening behavior

The config file will distinguish:

- `sync_listen`
- `admin_listen`

Recommended defaults:

```toml
sync_listen = ["0.0.0.0:8787", "[::]:8787"]
admin_listen = ["127.0.0.1:8788", "[::1]:8788"]
```

This keeps sync reachable by remote clients while keeping the admin console local-only by default.

## Configuration Model

The server continues to auto-create a config file beside the executable, but the schema will expand.

### Config file path

- default: `sync-server.toml` in the executable directory
- override: `--config /path/to/sync-server.toml`

### Config schema

```toml
sync_listen = ["0.0.0.0:8787", "[::]:8787"]
admin_listen = ["127.0.0.1:8788", "[::1]:8788"]
db_path = "data/floral-sync.sqlite3"
export_dir = "exports"
log_path = "logs/floral-sync-server.log"
log_level = "info"
sync_token = "replace-this-token"
admin_password_hash = "$argon2id$..."
admin_session_secret = "random-secret-for-cookie-signing"
```

### Config compatibility and migration

The server must migrate existing configs without breaking current installs:

1. Legacy `listen = [...]` becomes `sync_listen = [...]`.
2. Legacy `bind = "..."` becomes `sync_listen = ["..."]`.
3. Legacy `token = "..."` becomes `sync_token = "..."`.
4. If `admin_listen` is missing, create local-only defaults.
5. If `admin_password_hash` is missing, create a bootstrap state where the admin console requires password setup on first login.
6. If `admin_session_secret` is missing, generate and persist a random secret.

### Immediate vs restart-required settings

**Immediate effect**

- `sync_token`
- `admin_password_hash`
- `admin_session_secret`
- `log_level`

**Restart required**

- `sync_listen`
- `admin_listen`
- `db_path`
- `export_dir`
- `log_path`

The admin console must clearly mark which saved settings require a restart.

## Authentication and Security

### Sync authentication

The sync surface remains unchanged:

- every sync request must send `Authorization: Bearer <sync_token>`
- unauthorized requests return `401`

### Admin authentication

The admin console uses a dedicated password:

1. Password is stored as an `Argon2id` hash in the config file.
2. Login happens through `POST /login`.
3. A successful login returns an `HttpOnly` session cookie.
4. All admin routes require a valid session.
5. Logout invalidates the session cookie.

### Admin bootstrap flow

If the config file does not yet contain `admin_password_hash`, the admin console starts in bootstrap mode:

1. `GET /admin/api/session` returns `bootstrapRequired = true`.
2. The browser shows a first-run password setup screen instead of the normal login screen.
3. `POST /admin/api/bootstrap` accepts the first admin password and writes the hash to config.
4. After bootstrap succeeds, the server creates a normal logged-in admin session.

Bootstrap is single-use. Once `admin_password_hash` exists, the bootstrap endpoint is disabled.

### Session model

Sessions are signed with `admin_session_secret`. The signed session payload contains:

- session id
- issued time
- expiry time
- optional password version marker

Sessions expire after a fixed timeout, for example 12 hours. Changing the admin password must invalidate old sessions by bumping the password version or rotating the session secret.

### CSRF and browser safety

The admin console uses cookie authentication, so state-changing admin APIs must also validate request origin:

- require `Origin` to match the admin host for `POST` requests
- mark the session cookie as `HttpOnly`, `SameSite=Strict`, and `Secure` when served behind HTTPS

### Read-only note policy

The admin UI never exposes note write operations. Note-related admin APIs only support:

- list
- detail
- history
- markdown download

No browser endpoint may mutate note content, deletion state, or revision state.

## Storage Design

### Existing table

The existing `notes` table remains the latest-state source used by sync responses. It continues to store:

- current note body
- category
- timestamps
- tombstone state
- content hash
- device id
- latest revision

### New snapshot table

Add a `note_snapshots` table to preserve accepted server history for read-only backup browsing.

Suggested schema:

```sql
CREATE TABLE note_snapshots (
    snapshot_id INTEGER PRIMARY KEY AUTOINCREMENT,
    note_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    category TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    content_hash TEXT NOT NULL,
    device_id TEXT NOT NULL,
    captured_at TEXT NOT NULL
);

CREATE INDEX note_snapshots_note_revision_idx
ON note_snapshots(note_id, revision DESC);
```

### Snapshot write behavior

When the server accepts a new winning state in `push()`:

1. Update the `notes` table as it does today.
2. Insert one new row into `note_snapshots` for that accepted state.

Outdated writes that lose the server-side timestamp check do not create snapshots.

### Why keep both tables

- `notes` stays fast and simple for sync.
- `note_snapshots` gives the web UI historical backup visibility.
- The client protocol stays unchanged because history is strictly an admin-side concern.

## Module Layout

The service should be refactored into smaller modules instead of growing `main.rs`.

### Rust modules

- `src/main.rs`
  - startup
  - CLI parsing
  - config loading
  - listener creation
- `src/config.rs`
  - config schema
  - migration
  - persistence
  - bootstrap helpers
- `src/auth.rs`
  - bearer token auth
  - admin password verification
  - cookie/session validation
- `src/session.rs`
  - session signing
  - session parsing
  - session expiry
- `src/sync_api.rs`
  - `/health`
  - `/v1/changes`
  - `/v1/push`
- `src/admin_api.rs`
  - login
  - logout
  - overview
  - notes
  - settings
  - maintenance
- `src/admin_web.rs`
  - embedded static file serving
  - `/admin` entry route
- `src/store/mod.rs`
  - shared store exports
- `src/store/sync_store.rs`
  - revision-driven sync writes and reads
- `src/store/admin_store.rs`
  - stats
  - note listing
  - snapshot history
  - backup metadata
- `src/protocol.rs`
  - existing sync protocol structs
- `src/logging.rs`
  - tracing subscriber
  - file log setup

### Frontend modules

- `admin-ui/`
  - `src/main.tsx`
  - `src/App.tsx`
  - `src/pages/LoginPage.tsx`
  - `src/pages/OverviewPage.tsx`
  - `src/pages/NotesPage.tsx`
  - `src/pages/SettingsPage.tsx`
  - `src/pages/MaintenancePage.tsx`
  - `src/components/*`
  - `src/lib/api.ts`
  - `src/lib/types.ts`

## Web Admin UI Design

The admin UI should feel like a compact modern operations console rather than a marketing site.

### Visual direction

- dense but readable
- dark or neutral control-room style
- strong data hierarchy
- restrained accent colors
- fast scanning for note lists and service status

### Navigation

The console uses a left sidebar with four top-level destinations:

1. Overview
2. Notes
3. Settings
4. Maintenance

The first screen after login is Overview.

### Page: Overview

Shows:

- sync service status
- admin service status
- current revision
- database path
- active sync listen addresses
- active admin listen addresses
- note counts
- deleted note counts
- category counts
- recently updated notes count
- latest accepted server write time
- restart-required configuration warning if pending changes exist

### Page: Notes

Shows a searchable, filterable table with:

- title
- category
- updated time
- deleted state
- source device
- latest revision

Interactions:

- search by title or content snippet
- filter by category
- filter active vs deleted
- open a detail drawer or detail page

Note detail view shows:

- title
- category
- content
- created time
- updated time
- deleted time
- content hash
- device id
- revision
- snapshot history timeline
- download as Markdown button

No edit controls appear anywhere in this flow.

### Page: Settings

Shows editable server settings:

- sync listen addresses
- admin listen addresses
- database path
- export directory
- log path
- log level
- sync token

Also shows:

- restart-required badge for listen and path changes
- immediate-effect badge for token or password changes
- explicit save confirmation

Password change should be handled in a dedicated form with:

- current password
- new password
- confirm new password

### Page: Maintenance

Shows:

- export SQLite backup button
- generated backup file list
- recent server logs
- rotate sync token button
- logout current session button

Log viewing is read-only and paged to avoid loading a giant file into the browser.

## Admin HTTP API

### Authentication endpoints

- `POST /login`
  - request: password
  - response: session cookie, bootstrap status
- `POST /logout`
  - clears session cookie
- `GET /admin/api/session`
  - returns logged-in session state

### Overview endpoints

- `GET /admin/api/overview`
  - revision
  - counts
  - active listeners
  - database path
  - recent activity summary

### Notes endpoints

- `GET /admin/api/notes`
  - query params: `page`, `pageSize`, `search`, `category`, `state`
- `GET /admin/api/notes/:id`
  - current note state
- `GET /admin/api/notes/:id/history`
  - snapshot timeline
- `GET /admin/api/notes/:id/download`
  - markdown download

### Settings endpoints

- `GET /admin/api/settings`
  - returns non-secret settings plus masked secret metadata
- `POST /admin/api/settings`
  - updates editable settings
- `POST /admin/api/settings/token/reset`
  - generates and persists a new sync token
- `POST /admin/api/settings/password`
  - changes admin password

Sensitive values must not be returned in plaintext from `GET /admin/api/settings`. The UI should receive masked status only, such as:

- whether a sync token exists
- whether an admin session secret exists
- the last rotation time if tracked

When the operator wants to replace the sync token manually, the new value is submitted one-way from the form and is never echoed back in API responses.

### Maintenance endpoints

- `POST /admin/api/maintenance/backup`
  - creates a timestamped SQLite backup in `export_dir`
- `GET /admin/api/maintenance/backups`
  - lists available backup files
- `GET /admin/api/logs`
  - returns recent server log lines

## CLI Behavior

The existing CLI keeps working, but config commands must understand the new schema.

Supported behaviors:

- `floral-sync-server`
- `floral-sync-server --config /path/to/file.toml`
- `floral-sync-server config show`
- `floral-sync-server config set ...`

Config set must be extended to support both listen surfaces:

- `--sync-listen`
- `--admin-listen`
- `--db`
- `--export-dir`
- `--log-path`
- `--log-level`
- `--token`
- `--generate-token`
- bootstrap helpers for admin password initialization

## Build and Embedding Strategy

### Admin UI build

The admin UI is built with Vite into static assets under `admin-ui/dist/`.

During release and dev builds:

1. build the admin UI
2. embed the generated files into the Rust binary
3. serve them from the admin router

Recommended embedding approaches are:

- `rust-embed`
- or `include_dir`

The selected implementation should prefer the simpler option with fewer custom build steps. `rust-embed` is a good default choice for this project.

### Release artifacts

The release process produces:

- `floral-sync-server.exe` for Windows
- `floral-sync-server` for Linux GNU
- `floral-sync-server` for Linux musl

### Cross compilation

Preferred tooling for Linux cross targets from Windows:

- `cargo-zigbuild`

Targets:

- `x86_64-unknown-linux-gnu`
- `x86_64-unknown-linux-musl`

If the local machine cannot produce Linux artifacts directly, the repository should still include scripts and documentation that make the intended release path explicit, with GitHub Actions as the long-term reliable path.

## Logging and Backup

### Logging

The server should switch from only console output to structured file logging with `tracing`.

Requirements:

- log to stdout and to file
- honor `log_level`
- write to `log_path`
- allow the admin UI to read a safe recent slice of the log file

### Backup export

The maintenance page can trigger a safe database copy into `export_dir` using a timestamped filename such as:

`floral-sync-2026-05-19T15-30-00.sqlite3`

The export action does not alter note content and therefore remains within the read-only admin model.

## API Compatibility Rules

The following compatibility guarantees are mandatory:

1. `GET /health` request and response shape do not change.
2. `GET /v1/changes` request and response shape do not change.
3. `POST /v1/push` request and response shape do not change.
4. Existing client bearer-token authentication behavior remains valid.
5. Existing revision semantics remain valid.
6. Existing last-write-wins server acceptance behavior remains valid.

Internal refactors may reorganize code, storage helpers, and config layout, but they must not change the client contract.

## Testing Strategy

### Rust unit tests

- config migration from `listen` and `bind`
- config defaults for `admin_listen`
- bootstrap admin password setup
- password hashing and verification
- session signing and expiry
- note snapshot insertion
- backup export path handling

### Rust integration tests

- existing sync endpoint compatibility
- sync endpoint auth failures
- admin login success and failure
- admin session requirement on protected routes
- notes list and detail are read-only
- token reset takes effect immediately
- password change invalidates old sessions
- settings save marks restart-required fields correctly

### Frontend tests

- login form
- overview stats rendering
- notes table filters
- note detail rendering
- settings save flows
- maintenance backup and log screens

### End-to-end validation

- desktop client syncs against the refactored server without code changes
- admin UI loads from the embedded assets
- Windows release binary serves sync and admin ports
- Linux GNU release binary serves sync and admin ports
- Linux musl release binary serves sync and admin ports

## Rollout Order

To reduce risk, implementation should proceed in this order:

1. Refactor the server into sync/admin/config/store/auth modules without changing sync behavior.
2. Expand the config schema and add migration for old config files.
3. Add admin password bootstrap and session auth.
4. Add the `note_snapshots` table and write-path integration.
5. Add the read-only admin API.
6. Build the React/Vite admin UI and embed it into the binary.
7. Add logging and backup export.
8. Add release scripts and validate Windows, Linux GNU, and Linux musl builds.
9. Run desktop client regression checks against the new server.

## Acceptance Criteria

The design is considered implemented successfully when all of the following are true:

1. The desktop client syncs with the new server without protocol changes.
2. The server exposes sync and admin surfaces on different configured ports.
3. The admin UI requires its own password login.
4. The admin UI can browse and download notes but cannot modify them.
5. The admin UI can update operational settings and clearly marks restart-required changes.
6. The admin UI can export backups and inspect recent logs.
7. The server builds into a Windows executable and Linux GNU plus musl binaries.
8. The `sync-server/` folder remains self-contained and can be extracted into a separate repository later.
