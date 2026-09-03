# Changelog

All notable user-facing changes to LibriSync. Entries are phrased for the
Google Play "What's new" field. Earlier entries were reconstructed from git
history (no changelog was kept before v0.0.24), so wording is approximate.

## v0.0.30

- Audible downloads are working normally again. The block Audible put on third-party
  apps on 3 September no longer affects LibriSync — books download through the standard
  path, and it keeps working after Audible retires the temporary fallback route.
- If your Audible downloads have been failing, sign out of your Audible account and sign
  in again — that is what applies the fix.

## v0.0.29

- Audible downloads work again. Audible began refusing download licences to every
  third-party app on 3 September; LibriSync now falls back to Audible's legacy download
  service, which is unaffected, and tells you plainly when it has done so.
- The fallback needs no setup and produces the same audio quality as before — the same
  file the licensed download would have given you.
- Fixed activation-byte retrieval, which had never worked. It is what makes the fallback
  possible.
- A download that is refused no longer reports a confusing file error.

## v0.0.28

- Download Format now applies to every store, not just Libro.fm. Choose one file per
  book, or a folder of MP3s split by chapter.
- Audible books can be saved as MP3s — LibriSync converts them on your device after
  the download, one file per chapter.
- Downloads show a "Converting" stage while MP3s are being written, and can be
  cancelled during it.
- Library sync is far harder to truncate: it fetches bigger pages, retries a page that
  fails instead of giving up on the rest of your library, and double-checks an empty
  response before deciding it has reached the end.
- A book Audible sends in an unexpected shape no longer takes its whole page with it —
  it is skipped and counted, and the rest of the page still syncs.
- Sync now tells you when it lost something, and "Copy Details" puts a page-by-page
  report on the clipboard. It is also available any time from Settings → Diagnostics.
- If the app hits an unrecoverable error it now shows a proper screen with a
  "Copy Crash Details" button instead of a blank one.
- When Audible temporarily throttles downloads, LibriSync now says so plainly instead
  of reporting a confusing file error, and stops asking for an hour rather than
  retrying into the same refusal.
- Fixed the download request itself, which did not match what Audible expects — the
  response it produced was missing the download link.
- Books with odd data from Audible (no purchase date, an unnamed author, a series with
  no id) no longer go missing from your library.
- Account details now load for regions where they previously showed a connection error.

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
