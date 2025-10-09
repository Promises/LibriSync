# LibriSync - Quick Start Guide

**Complete audiobook download and decryption pipeline working!**

---

## 🚀 Quick Commands

### Download + Decrypt a Book (Complete Pipeline)
```bash
cd /Users/henningberge/Documents/projects/rn-audible/native/rust-core

# Complete download and decrypt
cargo run --example download_and_decrypt

# Output: /tmp/book_decrypted.m4b (playable M4B file)
```

### Test Individual Components

```bash
# 1. Refresh authentication tokens
cargo run --example fetch_my_books

# 2. Test license request (gets download URL and keys)
cargo test --lib test_download_book_b07t2f8vjm -- --ignored --nocapture

# 3. Download encrypted file
cargo run --example test_download_b07t2f8vjm

# 4. Test resume capability
cargo run --example test_download_resume

# 5. Run all unit tests
cargo test --lib
```

---

## 📋 What Works Now

### ✅ Complete Pipeline (End-to-End)
1. **Authenticate** with Audible OAuth
2. **Request license** for book → Get download URL + DRM keys
3. **Download** encrypted AAXC file (150 MB) with progress
4. **Resume** downloads after interruption (HTTP 206)
5. **Decrypt license_response** (AES-128-CBC) → Extract keys
6. **Decrypt AAXC** file (FFmpeg) → Playable M4B
7. **Play** audiobook in any media player

### 🎯 Verified with Real Book
- **ASIN:** B07T2F8VJM
- **Title:** "A Grown-Up Guide to Dinosaurs"
- **Author:** Ben Garrod
- **Size:** 150 MB encrypted → 148 MB decrypted
- **Duration:** 2 hours 43 minutes
- **Format:** AAXC (16-byte key + 16-byte IV)
- **Playback:** ✅ Perfect (no decoding errors)

---

## 🔑 Decryption Keys (for B07T2F8VJM)

**AAXC Keys (hex-encoded):**
```
Key: 7ed05bd5f2e7babd20d984de15862181
IV:  d957c0f275895759e0da5aea7aecaa04
```

**Decrypt Command:**
```bash
ffmpeg -audible_key 7ed05bd5f2e7babd20d984de15862181 \
       -audible_iv d957c0f275895759e0da5aea7aecaa04 \
       -i /tmp/atomic_habits.aax \
       -c copy -vn \
       /tmp/atomic_habits_decrypted.m4b
```

**Play:**
```bash
mpv /tmp/atomic_habits_decrypted.m4b
# or
ffplay /tmp/atomic_habits_decrypted.m4b
```

---

## 📁 File Locations

### Downloaded Files
```
/tmp/atomic_habits.aax                    # 150 MB encrypted AAXC
/tmp/atomic_habits_aaxc_decrypted.m4b    # 148 MB playable M4B
/tmp/atomic_habits_resumable.aax          # 150 MB resume test
```

### Test Fixtures
```
test_fixtures/registration_response.json           # OAuth tokens (auto-updated)
test_fixtures/download_license_b07t2f8vjm.json    # License with keys
```

### Source Code
```
src/api/license.rs          # License API + AES decryption
src/download/               # Download manager
examples/download_and_decrypt.rs  # Complete pipeline
```

---

## 🔬 How It Works

### Step 1: License Request
```rust
let client = AudibleClient::new(account)?;
let license = client.build_download_license(
    "B07T2F8VJM",
    DownloadQuality::High,
    false  // Use AAXC, not Widevine
).await?;

// Returns:
// - download_url: CloudFront signed URL
// - license_response: AES-encrypted voucher blob
// - drm_type: Adrm
```

### Step 2: Decrypt license_response
```rust
// Derive AES key from device info
let key_components = device_type + device_serial + account_id + asin;
let hash = SHA256(key_components);
let aes_key = hash[0..16];   // First 16 bytes
let aes_iv = hash[16..32];   // Last 16 bytes

// Decrypt license_response with AES-128-CBC
let plaintext = aes_decrypt(license_response, aes_key, aes_iv);

// Parse decrypted JSON
let voucher = parse_json(plaintext);
// voucher.key = "7ed05bd5..." (32 hex chars = 16 bytes)
// voucher.iv = "d957c0f2..." (32 hex chars = 16 bytes)
```

### Step 3: Download File
```http
GET https://cloudfront.net/.../file.aax
Headers:
  User-Agent: Audible/671 CFNetwork/1240.0.4 Darwin/20.6.0

Response: 200 OK
Content-Length: 157642003
```

### Step 4: Decrypt AAXC → M4B
```bash
ffmpeg -audible_key {key_hex} \
       -audible_iv {iv_hex} \
       -i encrypted.aax \
       -c copy -vn \
       decrypted.m4b
```

---

## 🎓 Key Technical Details

### AAXC Encryption Scheme
**Reference:** AudibleApi.Common/ContentLicenseDtoV10.cs

```
1. license_response contains AES-encrypted voucher
2. AES key derived from: SHA256(deviceType + serial + accountId + asin)
3. Voucher JSON contains hex-encoded key + IV (16 bytes each)
4. FFmpeg uses these keys to decrypt audio streams
```

### Why Two Decryption Steps?
1. **First decrypt (license_response):** Get the AAXC keys
   - Uses device-specific AES key
   - Protects keys in transit

2. **Second decrypt (audio file):** Get playable audio
   - Uses extracted AAXC keys
   - Decrypts AAC audio streams

### File Format Detection
```rust
match (key_len, iv_len) {
    (4, None) => AAX,       // 4-byte activation bytes
    (16, Some(16)) => AAXC, // 16-byte key + IV
    _ => Unknown
}
```

---

## 🐛 Troubleshooting

### "Request could not be authenticated" (HTTP 403)
**Solution:** Refresh tokens
```bash
cargo run --example fetch_my_books
```

### "URL verification failed" (HTTP 403 on download)
**Issue:** Missing User-Agent header
**Solution:** Already handled in examples (see test_download_b07t2f8vjm.rs:77)

### Audio decoding errors in mpv
**Issue:** File still encrypted or wrong keys
**Check:**
1. Verify file type is AAXC (not AAX): `cargo test --ignored test_download_book_b07t2f8vjm`
2. Verify keys extracted: Look for "Key (AAXC)" and "IV (AAXC)" in test output
3. Use correct ffmpeg command: `-audible_key` and `-audible_iv` (not `-activation_bytes`)

### Wrong book title in metadata
**This is normal:** The downloaded file contains embedded metadata from Audible. The ASIN B07T2F8VJM is "A Grown-Up Guide to Dinosaurs", not "Atomic Habits"

---

## 📊 Performance Metrics

| Operation | Time | Bandwidth |
|-----------|------|-----------|
| License request | < 1s | ~1 KB |
| Download 150 MB | ~30s | ~5 MB/s |
| AES decrypt license | < 0.1s | - |
| FFmpeg decrypt | ~2s | ~75 MB/s |
| Total pipeline | ~33s | - |

---

## 🎯 Next Steps

### Immediate
- [x] Download working
- [x] Resume working
- [x] AAXC decryption working
- [x] Playback verified
- [ ] Integrate into React Native app
- [ ] Add UI progress bars
- [ ] Background downloads

### Future
- [ ] AAX support (4-byte activation bytes)
- [ ] Widevine/DASH support
- [ ] WiFi-only mode
- [ ] Download queue management
- [ ] Chapter navigation

---

## 📚 Documentation

- **IMPLEMENTATION_STATUS.md** - This file (overall status)
- **DOWNLOAD_SUCCESS_REPORT.md** - Detailed test results
- **DOWNLOAD_IMPLEMENTATION_STATUS.md** - Architecture details
- **QUICK_START.md** - This guide (quick reference)

---

## ✅ Success Checklist

- [x] OAuth authentication
- [x] Library API integration
- [x] License request API
- [x] AES-128-CBC decryption
- [x] AAXC key extraction
- [x] HTTP download with progress
- [x] HTTP resume (Range requests)
- [x] FFmpeg decryption
- [x] Playback verification
- [x] 126/126 unit tests passing
- [x] Integration tests with real book

---

**🎉 LibriSync can now download and decrypt Audible audiobooks just like Libation!**
