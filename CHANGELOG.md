# Changelog

All notable user-facing changes to LibriSync. Entries are phrased for the
Google Play "What's new" field. Earlier entries were reconstructed from git
history (no changelog was kept before v0.0.24), so wording is approximate.

## v0.0.27

- One Accounts tab for every store — Audible and Libro.fm accounts now live in a
  single list instead of behind a separate Providers screen.
- "Add Account" asks which store you want to connect.
- "Sync All Accounts" now syncs every store in one tap, not just one at a time.
- Browse LibriVox moved to a button in the top right of the Accounts screen.

## v0.0.26

- Libro.fm support — sign in and sync your owned library alongside Audible.
- Download Libro.fm books as one packaged M4B or as a folder of MP3 parts,
  your choice in Settings.
- Filter the library by source: Audible, LibriVox, or Libro.fm.
- Libro.fm gets the same multi-account handling as Audible — add several, sync each on its own.
- Each store's accounts are kept separate, so adding one no longer changes the other's screen.
- "Sync All Accounts" now only appears when a store actually has more than one.
- Library rows are less cluttered — the account name chip has been removed.
- Fixed a finished sync reporting a library total of zero.

## v0.0.25

- Auto-download new books after each library sync (optional, off by default, Wi-Fi only).
- Batch mode: select multiple books and download them all at once.
- "Stop All" — cancel every download, or every running process, in one tap.
- Cancel a download at any stage — downloading, decrypting, validating, or saving.
- Retry a failed download straight from its notification.
- Cancelling now cleans up partial files and never un-marks an already-downloaded book.

## v0.0.24

- Download multiple audiobooks at once, or switch to one-at-a-time in Settings.
- Live progress with speed and time-remaining for every stage — downloading,
  decrypting, and saving.
- Queued books are clearly shown and can be removed from the queue.
- Adjustable audio validation (Full / Quick / Off) for faster saves.
- Richer, more reliable download notifications with per-book pause and cancel.
- Various download stability fixes.

## v0.0.23

- Show progress, speed, and estimated time remaining for every download stage.
- Download several audiobooks at the same time.
- Fixed a stuck "Downloading…" notification that could linger after downloads finished.

## v0.0.22

- New library filters: genre, series, and type (audiobooks or podcasts), with multi-select.
- Tap a book for a detail card — narrators, series, length, release date, and full summary.
- "Sync All Accounts" for multi-account libraries, with account badges.
- Export your library to Goodreads / StoryGraph (CSV).
- Back up and restore your database from Settings.
- Security hardening and podcast/download reliability improvements.

## v0.0.21

- Added Demo Mode — explore the full app backed by free LibriVox books, no account needed.

## v0.0.20

- Fixed account registration for amazon.co.uk and other regional domains.

## v0.0.19

- Improved podcast episode handling, "download all", and download-queue reliability.

## v0.0.18

- Repaired Audible podcast downloads.
- Debug builds now install alongside the release app.

## v0.0.17

- More graceful handling of library sync errors.

## v0.0.16

- Require a download folder before downloading; added a "downloaded" sort.
- Automatically link audiobooks already on the device after a library sync.

## v0.0.15

- Unified LibriVox downloads with the main download pipeline.

## v0.0.14

- Added library export options (CSV, JSON, XLSX, TXT, PNG).

## v0.0.13

- Hardened Android library sync; added sort-by-length.

## v0.0.12

- Added LibriVox as a free public-domain audiobook source.

## v0.0.11

- Added an update check for GitHub releases.
- Hardened Android sync and download flows.

## v0.0.1 – v0.0.10

- Early testing and infrastructure releases: initial Audible sync, download and
  decrypt pipeline, FFmpeg-Kit integration, notifications, conversion retry, and
  the CI/release build system.
