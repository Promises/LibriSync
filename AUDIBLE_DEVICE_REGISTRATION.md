# Audible device registration — profiles, the 2026-09 throttle, and the deferred switcher

**Status:** LibriSync registers as **iPhone** and that is the shipped, working path (v0.0.30).
A settings switcher between iPhone and Android profiles is **deliberately deferred** — see
the decision below. This doc exists so the recipe and the reasoning survive; do not rely on
chat memory for it.

## TL;DR

- The 2026-09 "License Denied / CustomerThrottled" wall that hit every third-party Audible
  client was **not** an Amazon crackdown. Root cause (found by Mbucari, merged upstream as
  AudibleApi PR #84 → v14.1, shipped in the Libation app as v14.2): **the device serial was
  the wrong length.** The Audible app uses a **10-byte serial (20 hex chars)**; Libation/
  AudibleApi were generating a **20-byte serial (40 hex chars)**, and Amazon rejected it on
  the **Android** device type.
- The "iPhone fixes it, Android doesn't" pattern everyone observed was **incidental**: the
  iPhone/audible-cli path used a correct-length GUID serial and so dodged the bug. It was
  never really iPhone-vs-Android; it was serial length.
- **LibriSync was never exposed to the bug.** We register as iPhone with a 16-byte/32-hex
  serial (`generateDeviceSerial()` in `modules/expo-rust-bridge/index.ts`), which is exactly
  the iPhone/audible-cli (`Mkb79IPhone`) recipe. We never sent a 20-byte serial.

## The two profiles

| | **iPhone** (what we ship) | **Android** (upstream default) |
|---|---|---|
| `device_type` | `A2CZJZGLK2JJVM` | `A10KISP2GWF0E4` |
| Serial | GUID, 32 hex chars (16 bytes) — we generate 16 | **10 bytes / 20 hex chars** (the load-bearing detail) |
| Login surface | iOS (`amzn_audible_ios*`, maplanding on `amazon.TLD`, `forceMobileLayout=true`) | Android (`amzn_audible_android_aui*`, maplanding on `audible.TLD`, `disableLoginPrepopulate=1`) |
| Codecs available | **AAC-LC only** | AAC-LC **and xHE-AAC** |
| xHE-AAC / Widevine | No | Yes (requires a Widevine CDM decryption path) |
| Stability track record | Long-stable (mkb79/audible-cli) | Broke in 2026-09 via the serial bug |

**Why the codec column doesn't matter to us:** xHE-AAC is delivered under **Widevine DRM**,
which is a completely separate decryption path (Widevine CDM) that LibriSync does **not**
implement. Our pipeline is AAXC → activation-bytes/voucher decrypt → M4B/MP3. So regardless
of profile we are on the **AAC-LC/AAXC** track. The Android profile's only real advantage is
therefore unavailable to us anyway.

## Decision: defer the switcher (revisit only if the iPhone path breaks)

For Libation the switcher is genuinely useful — its users want xHE-AAC (Android/Widevine),
and a backup profile is a hedge after Amazon moved once. For **LibriSync the everyday
benefit is zero**: an Android registration would hand our users the same AAC-LC content we
already get on iPhone. The switcher's *only* value to us is **disaster recovery** — if Amazon
breaks the iPhone registration the way they broke Android, users could flip to the backup
without waiting for an app release.

That is a real but low-probability, not-currently-active risk, and the iPhone path is the
long-stable one. Against it, the cost is real (below). So: **keep iPhone as the working
default, build nothing now, and implement the switch only if/when Amazon actually breaks the
iPhone path** — at which point we have the exact recipe here and a concrete reason.

Upstream mirrors this "two profiles as mutual backups" stance (rmcrackan + Mbucari,
2026-09-07): keep both precisely so that if one device registration stops working, the other
is already available.

## If we DO build it later — scope and traps

A correct Android profile is **not a flag flip**. It requires replicating Mbucari's whole
recipe (AudibleApi PR #84). Getting any of it subtly wrong means it registers fine and *then*
throttles — the exact failure that started this.

1. **Rust (`native/rust-core/src/api/auth.rs`)** — parametrize registration by profile
   instead of the hardcoded iPhone constants:
   - `device_type` (`A10KISP2GWF0E4`), Android app/OS version strings, `device_model`,
     `software_version`, `os_version`.
   - The `device_metadata` block (iPhone omits it; Android includes it:
     `device_os_family`, `manufacturer`, `model`, `os_version`, `product`).
   - `auth_data.client_domain` / `registration_data.domain` = `DeviceLegacy`.
   - Android login surface in `generate_authorization_url`: `assoc_handle` =
     `amzn_audible_android_aui_{cc}`, `pageId` = `amzn_audible_android_aui_v2_dark_{cc}`
     (note: upstream fixed a doubled-`us` typo here — it is `_dark_{cc}`, not `_dark_us{cc}`),
     `return_to` = `https://www.audible.{TLD}/ap/maplanding`, `disableLoginPrepopulate=1`
     (no `forceMobileLayout`).
   - `user_context_map.frc` in the **registration body** (added in PR #84), plus the Android
     frc/map-md/sid sign-in cookies.
2. **Serial length** — the Android profile **must** use a **10-byte** serial (20 hex chars).
   Our `generateDeviceSerial()` produces 16 bytes; keep 16 for iPhone, use 10 for Android.
3. **Settings UI** — a picker (iPhone / Android). Not a live toggle: switching **forces a
   full re-registration** (sign out + sign in). Guard against users re-registering
   repeatedly and throttling themselves.
4. **Testing** — the only real validation is an on-device end-to-end re-registration against
   a **live account, for both profiles**: sync + license + AAXC download + decrypt. There is
   no unit-test substitute for "does Amazon grant the licence for this registration."

## Open question that could change all of this

As of 2026-09-07, at least one user (sschultz9999) did the full Android re-registration
(Libation 14.2, remove/re-add/re-login) and **still hit residual throttling** on older
purchases — on a **US/amazon** account, the same region/type as Mbucari's own test account.
Mbucari flagged it "concerning" and is on a wait-and-see; the deeper probe would be logging
into the Audible app via an Android emulator with the user's credentials to diff app-vs-
Libation behaviour.

If that turns out to be a genuine **account-specific throttle beyond device registration**,
then (a) building the Android switcher wouldn't have fixed it anyway, and (b) whatever the
real second cause is would be directly portable to us. This is why the hourly upstream watch
(rmcrackan/AudibleApi, rmcrackan/Libation, issue 2021) stays running.

## References

- AudibleApi PR #84 (Mbucari, "Fix Android device registration") — the root-cause fix.
- Libation issue 2021 ("License denied error message - unable to download").
- Our fix commit: `0eebd71` — register as the iPhone device type (v0.0.30).
- Serial generation: `generateDeviceSerial()` in `modules/expo-rust-bridge/index.ts`.
- Registration/auth: `native/rust-core/src/api/auth.rs`.
