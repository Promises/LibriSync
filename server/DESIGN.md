# LibriSync Server — design

A self-hosted, *arr-style daemon that keeps a local copy of every audiobook a user has
bought on Audible, Libro.fm and Chirp. It syncs each account's library on a schedule,
downloads new purchases promptly, back-fills the existing library slowly enough not to
trip Audible's licence throttle, and hands finished books to Audiobookshelf.

It is a **separate product built on the same Rust core** as the LibriSync app, not a fork:
`server/` is its own crate that depends on `native/rust-core` by path, so every Audible
profile fix, throttle fix or client-profile change lands in both at once.

## Shape

```
server/
  Cargo.toml            bin `librisync-server`, depends on ../native/rust-core
  src/                  axum HTTP API, scheduler, download queue, notifications
  web/                  React + Vite SPA (plain CSS, Nord palette), built into web/dist
                        and embedded in the binary with rust-embed
  Dockerfile            multi-stage: node (web) → rust (binary) → debian:trixie-slim + ffmpeg
  docker/entrypoint.sh  PUID/PGID/UMASK handling, then exec the binary under tini
```

Native core additions used by the server (both are plain Rust, no JNI, usable later by the
app and the iOS port):

- `rust_core::pipeline` — executes a `DownloadPlan` end to end with the system `ffmpeg`:
  resumable download, AAXC/AAX decrypt, ZIP extract, track join, validation, M4B or
  MP3-per-chapter output, tags, cover, Audiobookshelf sidecars. Stateless: no DB.
- `rust_core::api::device_code` — Amazon device-code ("code pair") Audible sign-in, plus
  `account_from_registration`, which builds the stored `Account` from a registration.
- `LibationError::error_code()` — stable machine codes (`license_throttled`, …).

## Runtime layout

| Path | What |
|---|---|
| `/config/librisync.db` | the rust-core database (Accounts, Books, …) — same schema as the app |
| `/config/server.db` | server-only state: settings, users, sessions, queue, history, cooldowns, notifications |
| `/config/logs/` | rolling log files |
| `/config/work/` | per-book scratch dirs (`work/<queue id>/`); partial downloads resume from here |
| `/audiobooks` | the library root (`LIBRISYNC_LIBRARY_DIR`) |

Env: `LIBRISYNC_CONFIG_DIR` (default `/config`), `LIBRISYNC_LIBRARY_DIR` (default
`/audiobooks`), `LIBRISYNC_PORT` (default `8383`), `LIBRISYNC_BIND` (default `0.0.0.0`),
`LIBRISYNC_LOG` (default `info`), `FFMPEG_PATH` / `FFPROBE_PATH` (default from `PATH`),
`PUID`/`PGID`/`UMASK`/`TZ` (entrypoint only).

The core DB keeps the core's own schema untouched. Server state lives in `server.db` with
its own migrations, so the server can never break the app's schema or vice versa.

## Accounts and sign-in

- **Audible, device code (primary).** `POST auth/create/codepair` → the UI shows
  "go to amazon.{tld}/code and enter **ABC12**" → the server polls `POST auth/register`
  with the code pair until it is confirmed (10 min). No password, PKCE or URL pasting;
  2FA/CAPTCHA/CVF happen on Amazon's page. Registers with the current iPhone profile.
- **Audible, external browser (fallback).** For pre-Amazon Audible accounts (legacy
  DE/US/UK "Audible username" accounts) that device code does not cover: the UI shows the
  sign-in URL, the user signs in and pastes the final `/ap/maplanding?...` URL back. The
  PKCE verifier and serial live in memory for the 15-minute session.
- **Libro.fm / Chirp:** email + password through `Provider::login`.
- **No import from the app.** The app has no account export, and sharing one device
  registration between the phone and the server would make two clients present as one
  device. The server registers its own device through the code sign-in instead.

## Sync

Per enabled account, every `syncIntervalHours` (default 24) with ±30 min random jitter,
and on demand. Pages are fetched with the same policy as the app's `runPagedSync`: three
attempts per page with backoff, skip a page that still fails, stop after three
consecutive failures, cap at 500 pages. Books whose `Books.created_at` is at or after the
sync's start are **new**.

The first successful sync of an account establishes its **baseline**: everything found
then is "existing library" (back-fill), not "new". Afterwards, new books are queued at
`new` priority if `autoDownloadNew` is on.

## Download queue

One queue, persisted in `server.db`. Priorities: `manual` (0) > `new` (1) > `backfill` (2).
A worker pool of `downloads.concurrency` (default 1) takes the best eligible item.

Each item runs: **plan** (provider `download_plan`, which for Audible is the licence
request) → **pipeline** → **import** (record the output, history event, notifications,
Audiobookshelf scan). Plans are not persisted across restarts: signed URLs expire, so a
restarted item re-plans. Partial files in its work dir are reused when the plan's part
file names match.

### Audible licence pacing (the point of the whole thing)

Only Audible licence requests are budgeted; DRM-free stores are limited by concurrency
alone. Nobody knows Audible's real threshold, so the defaults are conservative and every
number is a setting:

| Setting | Default |
|---|---|
| `audible.dailyLicenceCap` | 20 per account, rolling 24 h, counting every licence *request* (granted or not) |
| `audible.spacingMinutes` | random 20–60 between licence requests per account (`new`/`manual` ignore the spacing but still count) |
| `backfill.window` | optional local-time window (e.g. 01:00–07:00) for back-fill only |
| `backfill.order` | `newest_first` (by purchase date) or `oldest_first` |

At most one licence request per account is in flight at a time.

**Refusals** (modelled on Libation's backoff, lengthened for an unattended daemon):

| Refusal | Effect |
|---|---|
| `license_throttled` (CustomerThrottled) | **Account cooldown**: no licence requests for that account for 24 h, doubling per consecutive throttle, capped at 7 days. After it expires the next request is a single probe; a success resets the level. |
| `license_denied` for an eligibility reason | **Title backoff**: 1 day, doubling, capped at 30 days. Other titles continue. |
| Network / disk / ffmpeg failure | Normal retry: up to 3 attempts with 5 min → 30 min → 2 h waits, then `failed`. No cooldown. |

A manual download bypasses title backoff and spacing, but **not** an account cooldown
unless the request says `force: true` (the UI warns that retrying into a throttle is
believed to extend it).

## Output layout

Default folder template (Audiobookshelf-friendly, one folder per book, never a loose
file in an author folder): `{author}/{series}/{seriesSequence} - {title}` when the book
is in a series, else `{author}/{title}`. Tokens: `{author}` (first author), `{authors}`,
`{title}`, `{subtitle}`, `{series}`, `{seriesSequence}`, `{year}`, `{narrator}`,
`{asin}`, `{provider}`. Path components are sanitised (no `/ \ : * ? " < > |`, no trailing
dots/spaces, ≤ 120 bytes). The audio file is `{title}.m4b`; MP3 output is
`NN - Chapter Title.mp3`. Sidecars (on by default): `cover.jpg`, `desc.txt`,
`reader.txt`, `metadata.opf`.

## Notifications ("Connect")

- **Webhook:** Sonarr-style JSON `{eventType, instanceName, applicationUrl, …}`,
  configurable method, headers and per-event toggles.
- **Apprise:** apprise-api server URL + configuration key or stateless URLs.
- **Audiobookshelf:** URL + API key + library id; `POST /api/libraries/:id/scan` after
  each imported book (debounced to one scan per minute).

Events: `test`, `bookAdded`, `grab`, `download`, `downloadFailed`, `licenceThrottled`,
`authExpired`, `health`, `healthRestored`.

## Security

- First start with no user: the UI shows a setup page that creates the admin
  user (argon2 hash). Sessions are an `HttpOnly`, `SameSite=Lax` cookie backed by the
  `sessions` table.
- The API key (generated on first start, shown under Settings → General) authenticates
  `X-Api-Key` or `?apikey=`. `GET /ping` is anonymous.
- Account credentials live in `librisync.db` in `/config`. That volume is the secret;
  the README says so.

## Non-goals (for now)

LibriVox (not behind the Rust provider abstraction yet), Widevine/xHE-AAC, multi-user,
uploading to Audiobookshelf over its API, Postgres.
