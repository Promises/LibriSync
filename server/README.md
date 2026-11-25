# LibriSync Server

A self-hosted, Sonarr/Radarr-style service that keeps a local, DRM-free copy of every
audiobook you have bought on **Audible**, **Libro.fm** and **Chirp**.

- Syncs each account's library daily and downloads new purchases automatically.
- Back-fills the books you already own **slowly**, so it doesn't trip Audible's licence throttle.
- Writes one chaptered M4B per book, or one MP3 per chapter, in an Audiobookshelf-friendly
  layout with cover and metadata sidecars.
- Web UI covering the library, the download queue and history, missing books, accounts,
  settings, notifications and system status. There is also a REST API with an API key,
  plus webhook, Apprise and Audiobookshelf hooks.

It shares its Rust core with the [LibriSync](../README.md) Android app.

## Quick start

```yaml
services:
  librisync:
    image: ghcr.io/promises/librisync-server:latest
    environment:
      - PUID=1000
      - PGID=1000
      - TZ=Etc/UTC
    volumes:
      - ./config:/config
      - /path/to/audiobooks:/audiobooks
    ports:
      - "8383:8383"
    restart: unless-stopped
```

Open `http://<host>:8383`, create the admin user, then **Accounts → Add account**.

**`/config` holds your store credentials** (the Audible device registration, the
Libro.fm and Chirp tokens), so treat it as a secret and back it up. Point
Audiobookshelf's library at the same folder you mount as `/audiobooks`.

## Signing in

| Store | How |
|---|---|
| Audible (Amazon account) | **Sign in with a code.** The server shows a 5-letter code; enter it at `amazon.<tld>/code` on any device. Password, 2-step verification and CAPTCHAs all happen on Amazon's own page, so the server never sees your password. It registers its own device, separate from your phone. |
| Audible (older Audible-only account) | **Sign in through the browser.** Open the link, sign in, then paste the final "page not found" URL back. |
| Libro.fm, Chirp | Email and password. Chirp accounts that use Google or Apple sign-in have no password and can't be added. |

## How downloads are paced

Audible refuses licences ("CustomerThrottled") to accounts that request too many, and
the lockout can last days. Nobody outside Audible knows the real limit, so these
defaults are deliberately conservative and every number can be changed under
**Settings → Audible pacing**:

- at most **20 licence requests per account in any 24 hours**
- **20–60 minutes** between licence requests, at random
- at most one licence request per account at a time
- on a throttle refusal: **no requests for that account for 24 hours**, doubling on each
  further refusal (up to 7 days), then a single probe request
- new purchases go ahead of the back-fill; the back-fill can be limited to a nightly
  window

Libro.fm and Chirp have no such limit; their downloads are limited only by the download
concurrency setting.

## Configuration

| Env | Default | |
|---|---|---|
| `PUID` / `PGID` | `1000` | user the service runs as (it can also run with `--user`) |
| `UMASK` | `022` | |
| `TZ` | `Etc/UTC` | used for the back-fill window |
| `LIBRISYNC_PORT` | `8383` | |
| `LIBRISYNC_BIND` | `0.0.0.0` | |
| `LIBRISYNC_CONFIG_DIR` | `/config` | databases, logs, scratch space for downloads in progress |
| `LIBRISYNC_LIBRARY_DIR` | `/audiobooks` | finished books |
| `LIBRISYNC_LOG` | from the UI | e.g. `debug`; overrides the UI's log level |
| `LIBRISYNC_APPLICATION_URL` | — | public URL, used in notification payloads |

Everything else (folder template, output format, schedules, pacing) lives in the UI.

**Folder template tokens:** `{author}`, `{authors}`, `{title}`, `{subtitle}`, `{series}`,
`{seriesSequence}`, `{year}`, `{narrator}`, `{asin}`, `{provider}`.

## API

Send the API key (shown under **Settings → General**) as `X-Api-Key` or `?apikey=`.
`GET /ping` needs no key and is used by the Docker health check. The full contract is
in [API.md](API.md); the design and its reasoning are in [DESIGN.md](DESIGN.md).

## Building

```bash
# Image (from the repository root; the server depends on native/rust-core)
docker build -f server/Dockerfile -t librisync-server .

# Local development
cd server/web && npm ci && npm run dev     # UI on :5173, proxies /api to :8383
cd server && cargo run                     # needs ffmpeg ≥ 4.4 with libmp3lame on PATH
                                           # set LIBRISYNC_CONFIG_DIR / LIBRISYNC_LIBRARY_DIR
                                           # to writable folders
```

Release images are built by `.github/workflows/server-image.yml` when a `server-vX.Y.Z`
tag is pushed. Each architecture (amd64, arm64) builds natively on its own runner, and
the results are merged into one multi-arch image. App release tags (`vX.Y.Z`) don't
trigger it.

## Not supported (yet)

LibriVox; Audible's Widevine/xHE-AAC and Dolby Atmos editions; multiple users; Postgres.
