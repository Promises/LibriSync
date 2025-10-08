# Libation → Rust Direct Library Port Plan

## Overview

This document outlines the comprehensive plan for creating a **direct 1:1 port of Libation as a Rust library**. The goal is to translate Libation's C# codebase into a standalone Rust library (`libaudible`) that provides the same functionality as the original Libation desktop application, but as a library suitable for embedding in the RN Audible React Native mobile app.

**Key Principles:**
1. **Direct Port**: Maintain the same architecture, logic flow, and API surface as Libation
2. **Library-First**: Create a reusable Rust library, not a standalone application
3. **Reference Implementation**: Use `references/Libation/` as the authoritative source for all logic
4. **Feature Parity**: Implement all core Libation features (authentication, library sync, DRM removal, downloads)
5. **Mobile-Ready**: Design for embedding in React Native via JNI/FFI bindings

## Architecture Analysis

### Libation Component Structure

Based on analysis of `references/Libation/Source/`:

1. **AudibleUtilities** - Audible API integration and authentication
2. **AaxDecrypter** - DRM decryption and audio processing
3. **DataLayer** - SQLite database with Entity Framework Core
4. **FileLiberator** - Download and decrypt orchestration
5. **FileManager** - File system operations
6. **ApplicationServices** - Business logic services
7. **DtoImporterService** - Data import and synchronization

### Key Technologies in Libation

- **Authentication**: mkb79's AudibleApi library
- **Database**: Entity Framework Core + SQLite
- **DRM**: AAX/AAXC decryption with activation bytes
- **Audio**: FFmpeg for conversion/processing
- **HTTP**: Custom HTTP client with progress tracking

---

## Rust Library Architecture

**Goal:** Create `libaudible` - a direct Rust port of Libation's core functionality as a reusable library.

### Design Philosophy

This is a **direct translation**, not a reimagining:
- **Preserve Libation's architecture**: Same module boundaries and responsibilities
- **Match Libation's data models**: Port Entity Framework entities to Rust structs
- **Replicate Libation's logic**: Download flows, crypto algorithms, API patterns
- **Reference C# code**: Each Rust module should have a corresponding C# reference in `references/Libation/Source/`

### Crate Structure (Direct Mapping from Libation)

```
native/rust-core/              → Libation/Source/
├── Cargo.toml
├── build.rs
└── src/
    ├── lib.rs                 # Library entry point + public API
    ├── jni_bridge.rs          # Android JNI bindings (mobile-specific)
    ├── error.rs               # Unified error types
    │
    ├── api/                   → AudibleUtilities/
    │   ├── mod.rs
    │   ├── auth.rs            → Mkb79Auth.cs (OAuth, device registration)
    │   ├── client.rs          → ApiExtended.cs (HTTP client, retry logic)
    │   ├── library.rs         → (Library sync methods)
    │   ├── content.rs         → (Content metadata retrieval)
    │   └── license.rs         → (License/voucher handling)
    │
    ├── crypto/                → AaxDecrypter/ + AudibleUtilities/Widevine/
    │   ├── mod.rs
    │   ├── activation.rs      → ActivationBytes extraction
    │   ├── aax.rs             → AAX decryption logic
    │   ├── aaxc.rs            → AAXC decryption logic
    │   └── widevine.rs        → Widevine CDM integration
    │
    ├── download/              → AaxDecrypter/ + FileLiberator/
    │   ├── mod.rs
    │   ├── manager.rs         → DownloadDecryptBook.cs (orchestration)
    │   ├── stream.rs          → NetworkFileStream.cs (streaming)
    │   └── progress.rs        → AverageSpeed.cs (progress tracking)
    │
    ├── audio/                 → FileLiberator/
    │   ├── mod.rs
    │   ├── decoder.rs         → AudioFormatDecoder.cs
    │   ├── converter.rs       → ConvertToMp3.cs (FFmpeg integration)
    │   └── metadata.rs        → (ID3 tags, cover art embedding)
    │
    ├── storage/               → DataLayer/
    │   ├── mod.rs
    │   ├── database.rs        → LibationContext.cs (database context)
    │   ├── models.rs          → EfClasses/ (Book, LibraryBook, Series, etc.)
    │   ├── migrations.rs      → Migrations/ (schema migrations)
    │   └── queries.rs         → QueryObjects/ (query helpers)
    │
    └── file/                  → FileManager/
        ├── mod.rs
        ├── manager.rs         → File system operations
        └── paths.rs           → Path utilities, naming templates
```

**Mapping Legend:**
- `→` indicates the Rust module ports functionality from the corresponding Libation C# component
- Each Rust file should reference its C# source counterpart in code comments
- Preserve Libation's class/method names where possible (converting to Rust naming conventions)

### Library API Surface

The Rust library will expose a public API matching Libation's core functionality:

```rust
// Equivalent to Libation's main operations
pub struct LibationLib {
    api_client: ApiClient,
    database: Database,
}

impl LibationLib {
    // Account management (→ AudibleUtilities)
    pub fn authenticate(email: &str, password: &str) -> Result<Account>;
    pub fn get_activation_bytes(account: &Account) -> Result<Vec<u8>>;

    // Library operations (→ AudibleUtilities + DataLayer)
    pub fn sync_library(account: &Account) -> Result<Vec<Book>>;
    pub fn get_library_books() -> Result<Vec<LibraryBook>>;

    // Download & Decrypt (→ FileLiberator + AaxDecrypter)
    pub fn download_book(asin: &str, options: DownloadOptions) -> Result<DownloadResult>;
    pub fn decrypt_aax(input: &Path, output: &Path, activation_bytes: &[u8]) -> Result<()>;

    // Audio processing (→ FileLiberator)
    pub fn convert_to_m4b(input: &Path, output: &Path) -> Result<()>;
}
```

This API will be wrapped with JNI/FFI for React Native integration.

---

## Porting Methodology

### Step-by-Step Translation Process

For each Libation C# class/module:

1. **Locate the C# source** in `references/Libation/Source/`
2. **Read and understand** the complete C# implementation
3. **Create Rust equivalent** with identical functionality
4. **Add reference comment** at top of Rust file:
   ```rust
   //! Direct port of Libation's XYZ functionality
   //! Reference: references/Libation/Source/ComponentName/ClassName.cs
   ```
5. **Port data structures** first (classes → structs)
6. **Port logic** method-by-method
7. **Write equivalent tests** based on Libation's test suite
8. **Validate behavior** matches original

### Example: Porting a C# Class

**C# Original** (`Libation/Source/AudibleUtilities/Account.cs`):
```csharp
public class Account {
    public string Email { get; set; }
    public string LocaleName { get; set; }
    public Identity Identity { get; set; }

    public async Task<Library> GetLibraryAsync() { ... }
}
```

**Rust Port** (`src/api/account.rs`):
```rust
//! Direct port of Libation's Account functionality
//! Reference: references/Libation/Source/AudibleUtilities/Account.cs

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub email: String,
    pub locale_name: String,
    pub identity: Identity,
}

impl Account {
    pub async fn get_library(&self) -> Result<Library> {
        // Port the exact logic from GetLibraryAsync()
        // ...
    }
}
```

### Porting Priorities

**Port in this order:**
1. **Data models** (easiest, no logic)
2. **Utilities** (pure functions, no side effects)
3. **Business logic** (stateful operations)
4. **Complex flows** (orchestration, async operations)

---

## Porting Phases

### Phase 1: Core Infrastructure (Week 1-2)

**Priority: Critical**

#### 1.1 Error Handling & Types
- [ ] Define error types (API, Network, Crypto, Storage)
- [ ] Create Result types for all operations
- [ ] Implement error propagation strategy

**Dependencies:**
```toml
thiserror = "2.0"      # Error types
anyhow = "1.0"         # Error context
```

#### 1.2 HTTP Client
- [ ] Async HTTP client with retry logic
- [ ] Cookie jar for session management
- [ ] Custom headers (User-Agent, device info)
- [ ] Progress tracking for downloads

**Dependencies:**
```toml
reqwest = { version = "0.12", features = ["json", "cookies", "stream"] }
tokio = { version = "1", features = ["full"] }
```

**Reference:** `Libation/Source/AudibleUtilities/ApiExtended.cs`

#### 1.3 Database Layer
- [ ] SQLite schema migration from Libation
- [ ] Define Rust models matching C# entities:
  - `Book` (title, ASIN, authors, narrators, etc.)
  - `LibraryBook` (user's library entry)
  - `UserDefinedItem` (download status, format)
  - `Series`, `Contributor`, `Category`
- [ ] CRUD operations
- [ ] Query helpers for common operations

**Dependencies:**
```toml
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

**Reference:** `Libation/Source/DataLayer/EfClasses/`

---

### Phase 2: Authentication & API (Week 3-4)

**Priority: Critical**
**Status:** ✅ **COMPLETE** (OAuth working in React Native app!)

#### 2.1 Audible Authentication
- [x] Implement OAuth 2.0 flow
- [x] Device registration (generate device keys)
- [x] Token management (access, refresh)
- [ ] Activation bytes retrieval (endpoint identified, binary extraction needed)
- [x] Account persistence

**Key Components:**
- OAuth flow with Audible
- RSA key generation for device
- Token refresh logic
- Secure storage of credentials

**Dependencies:**
```toml
oauth2 = "4.4"
rsa = "0.9"
rand = "0.8"
base64 = "0.22"
```

**Reference:**
- `Libation/Source/AudibleUtilities/Mkb79Auth.cs`
- `Libation/Source/AudibleUtilities/Account.cs`

#### 2.2 Library Sync
- [ ] Fetch library (paginated)
- [ ] Parse library response (JSON → Models)
- [ ] Incremental sync (detect changes)
- [ ] Store in database

**API Endpoints:**
```
GET /1.0/library?num_results=1000&page=1
GET /1.0/library/{asin}
GET /1.0/content/{asin}/metadata
```

**Reference:** `Libation/Source/AudibleUtilities/ApiExtended.cs`

#### 2.3 Content Metadata
- [ ] Fetch detailed book metadata
- [ ] Retrieve content license/voucher
- [ ] Get download URL with authentication
- [ ] Cover art URLs

**Reference:** `Libation/Source/FileLiberator/DownloadOptions.Factory.cs`

---

### Phase 3: DRM & Decryption (Week 5-7)

**Priority: High**

#### 3.1 Activation Bytes
- [ ] Extract activation bytes from account
- [ ] Derive decryption keys
- [ ] Store securely

**Dependencies:**
```toml
sha1 = "0.10"
hmac = "0.12"
```

**Reference:** `Libation/Source/AudibleUtilities/Mkb79Auth.cs` (ActivationBytes)

#### 3.2 AAX Decryption
- [ ] Parse AAX file structure
- [ ] Decrypt audio chunks using activation bytes
- [ ] Stream decrypted audio

**Format:** AAX is encrypted MP4/M4B
**Algorithm:** AES-128-CBC

**Dependencies:**
```toml
aes = "0.8"
cbc = "0.1"
```

**Reference:**
- `Libation/Source/AaxDecrypter/AudiobookDownloadBase.cs`
- Look for FFmpeg usage for actual decryption

#### 3.3 AAXC Decryption (Widevine)
- [ ] Parse AAXC manifest (JSON)
- [ ] Extract Widevine PSSH box
- [ ] CDM integration for key retrieval
- [ ] Decrypt audio chunks

**Format:** AAXC uses Widevine DRM (more complex than AAX)

**Dependencies:**
```toml
# May need to use FFI to widevine CDM or pywidevine
```

**Reference:** `Libation/Source/AudibleUtilities/Widevine/`

**Note:** Widevine is complex. Consider:
1. Using pywidevine via FFI
2. Or implementing Widevine CDM in Rust (significant effort)

---

### Phase 4: Download & Processing (Week 8-10)

**Priority: High**

#### 4.1 Download Manager
- [ ] Parallel download chunks
- [ ] Resume capability
- [ ] Progress tracking (bytes, percentage)
- [ ] Error handling and retry
- [ ] Queue management

**Reference:** `Libation/Source/AaxDecrypter/NetworkFileStream.cs`

#### 4.2 Audio Processing
- [ ] Decrypt AAX/AAXC while downloading
- [ ] Convert to M4B/MP3 (using FFmpeg)
- [ ] Embed metadata (ID3 tags)
- [ ] Add cover art
- [ ] Generate chapters

**Dependencies:**
```toml
# FFmpeg bindings
ffmpeg-next = "7.0"
# Or use command-line ffmpeg
```

**Reference:**
- `Libation/Source/FileLiberator/ConvertToMp3.cs`
- `Libation/Source/FileLiberator/AudioFormatDecoder.cs`

#### 4.3 File Management
- [ ] Output directory management
- [ ] Filename templates (author, title, series)
- [ ] Move completed files
- [ ] Cleanup temp files

**Reference:** `Libation/Source/FileManager/`

---

### Phase 5: React Native Integration (Week 11-12)

**Priority: High**

#### 5.1 JNI Bridge Expansion
- [ ] Account management functions
- [ ] Library sync functions
- [ ] Download control (start, pause, cancel)
- [ ] Progress callbacks
- [ ] Settings management

**Example JNI Functions:**
```rust
// Account
#[no_mangle]
pub extern "C" fn Java_..._authenticateAudible(env, email, password) -> JString
#[no_mangle]
pub extern "C" fn Java_..._getActivationBytes(env) -> JString

// Library
#[no_mangle]
pub extern "C" fn Java_..._syncLibrary(env) -> JString
#[no_mangle]
pub extern "C" fn Java_..._getBooks(env) -> JString

// Download
#[no_mangle]
pub extern "C" fn Java_..._downloadBook(env, asin) -> JString
#[no_mangle]
pub extern "C" fn Java_..._getDownloadProgress(env, asin) -> JString
```

#### 5.2 iOS Bridge (C FFI)
- [ ] C header generation
- [ ] Swift/Objective-C Expo module
- [ ] Same API surface as Android

**Reference:** Use uniffi for automatic binding generation

---

### Phase 6: Advanced Features (Week 13-15)

**Priority: Medium**

#### 6.1 Batch Operations
- [ ] Download entire library
- [ ] Series-aware ordering
- [ ] Smart download (only new books)

#### 6.2 Background Processing
- [ ] Background download (Android WorkManager)
- [ ] Background download (iOS Background Tasks)
- [ ] Notification updates

#### 6.3 PDF Supplements
- [ ] Download PDF companions
- [ ] Metadata storage

**Reference:** `Libation/Source/FileLiberator/DownloadPdf.cs`

---

## Key Dependencies Summary

```toml
[dependencies]
# Core
tokio = { version = "1", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "2.0"
anyhow = "1.0"

# HTTP & API
reqwest = { version = "0.12", features = ["json", "cookies", "stream"] }
oauth2 = "4.4"

# Database
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite"] }

# Cryptography
rsa = "0.9"
aes = "0.8"
cbc = "0.1"
sha1 = "0.10"
hmac = "0.12"
rand = "0.8"
base64 = "0.22"

# Audio Processing
ffmpeg-next = "7.0"  # or use CLI

# Mobile Bindings
jni = "0.21"
uniffi = "0.28"
```

---

## Critical Challenges

### 1. Widevine CDM for AAXC
**Problem:** Widevine is proprietary and complex
**Solutions:**
- Use pywidevine via FFI
- Implement minimal CDM in Rust (reverse-engineer)
- Only support AAX initially, add AAXC later

### 2. FFmpeg Integration
**Problem:** Audio conversion requires FFmpeg
**Solutions:**
- Use `ffmpeg-next` crate (Rust bindings)
- Shell out to ffmpeg CLI
- Use libav libraries directly

### 3. Cross-Platform File Systems
**Problem:** Android storage permissions, iOS sandboxing
**Solutions:**
- Use Expo FileSystem API
- Request runtime permissions
- Scoped storage on Android 11+

### 4. Background Downloads
**Problem:** Mobile OS restrictions
**Solutions:**
- Android: WorkManager
- iOS: Background Tasks framework
- Requires native bridge for each platform

---

## Comprehensive Testing Strategy

### Test Pyramid Structure

```
                    /\
                   /  \          E2E Tests (5%)
                  /____\         - Full user workflows on devices
                 /      \
                /________\       Integration Tests (25%)
               /          \      - Component interactions
              /____________\
             /              \    Unit Tests (70%)
            /________________\   - Individual functions/modules
```

### 1. Unit Tests (70% of test coverage)

#### 1.1 Error Handling (`src/error.rs`)
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_api_error_from_status_code() { }

    #[test]
    fn test_error_chain_context() { }

    #[test]
    fn test_crypto_error_variants() { }
}
```

**Coverage:**
- [ ] All error variants constructible
- [ ] Error messages contain useful context
- [ ] Error chains preserve root cause
- [ ] Serialization/deserialization works

#### 1.2 HTTP Client (`src/api/client.rs`)
```rust
#[cfg(test)]
mod tests {
    use mockito::{mock, server_url};

    #[tokio::test]
    async fn test_http_get_success() { }

    #[tokio::test]
    async fn test_http_retry_on_failure() { }

    #[tokio::test]
    async fn test_http_timeout() { }

    #[tokio::test]
    async fn test_cookie_persistence() { }
}
```

**Coverage:**
- [ ] Successful requests
- [ ] Retry logic (exponential backoff)
- [ ] Timeout handling
- [ ] Cookie jar management
- [ ] Custom headers
- [ ] Error responses (4xx, 5xx)

**Tools:**
```toml
mockito = "1.5"          # HTTP mocking
wiremock = "0.6"         # HTTP stubbing
```

#### 1.3 Database Operations (`src/storage/`)
```rust
#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_create_book() { }

    #[tokio::test]
    async fn test_query_books_by_author() { }

    #[tokio::test]
    async fn test_update_download_status() { }

    #[tokio::test]
    async fn test_database_migration() { }

    #[tokio::test]
    async fn test_concurrent_writes() { }
}
```

**Coverage:**
- [ ] CRUD operations for all entities
- [ ] Complex queries (joins, filters)
- [ ] Transactions and rollbacks
- [ ] Schema migrations
- [ ] Concurrent access
- [ ] Foreign key constraints

**Tools:**
```toml
tempfile = "3.13"        # Temporary test databases
```

#### 1.4 Cryptography (`src/crypto/`)
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_activation_bytes_derivation() {
        // Test vectors from known accounts
    }

    #[test]
    fn test_aax_key_extraction() { }

    #[test]
    fn test_aes_decrypt_chunk() {
        // Use sample encrypted data
    }

    #[test]
    fn test_widevine_pssh_parsing() { }
}
```

**Coverage:**
- [ ] Activation bytes calculation (known test vectors)
- [ ] AES encryption/decryption
- [ ] Key derivation functions
- [ ] AAX header parsing
- [ ] AAXC manifest parsing

#### 1.5 File Operations (`src/file/`)
```rust
#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    #[test]
    fn test_filename_template() {
        assert_eq!(
            generate_filename("{author} - {title}", book),
            "John Doe - Sample Book"
        );
    }

    #[test]
    fn test_safe_filename_sanitization() { }

    #[test]
    fn test_move_file_atomic() { }
}
```

**Coverage:**
- [ ] Filename template expansion
- [ ] Path sanitization
- [ ] Atomic file moves
- [ ] Directory creation
- [ ] Cleanup operations

---

### 2. Integration Tests (25% of test coverage)

#### 2.1 Authentication Flow (`tests/integration/auth.rs`)
```rust
#[tokio::test]
#[ignore] // Requires network
async fn test_oauth_flow_end_to_end() {
    // Use test account credentials
    let client = ApiClient::new();
    let auth = client.authenticate(email, password).await.unwrap();

    assert!(auth.access_token.is_some());
    assert!(auth.refresh_token.is_some());
    assert!(auth.activation_bytes.is_some());
}

#[tokio::test]
#[ignore]
async fn test_token_refresh() { }
```

**Test Requirements:**
- [ ] Test Audible account (disposable)
- [ ] Mock OAuth server (for offline tests)
- [ ] Token expiration simulation

#### 2.2 Library Sync (`tests/integration/library.rs`)
```rust
#[tokio::test]
#[ignore]
async fn test_sync_full_library() {
    let client = authenticated_client().await;
    let books = client.sync_library().await.unwrap();

    assert!(books.len() > 0);
    // Verify database contains synced books
}

#[tokio::test]
async fn test_incremental_sync() { }

#[tokio::test]
async fn test_sync_with_pagination() { }
```

**Coverage:**
- [ ] Initial sync (empty database)
- [ ] Incremental sync (updates only)
- [ ] Pagination handling (>1000 books)
- [ ] Error recovery (network failure mid-sync)

#### 2.3 Download & Decrypt (`tests/integration/download.rs`)
```rust
#[tokio::test]
#[ignore]
async fn test_download_aax_and_decrypt() {
    let temp_dir = TempDir::new().unwrap();
    let book = get_test_book_asin();

    let result = download_and_decrypt(
        book,
        temp_dir.path(),
        |progress| {
            println!("Progress: {}%", progress);
        }
    ).await.unwrap();

    assert!(result.output_file.exists());
    assert_eq!(result.format, AudioFormat::M4B);
}

#[tokio::test]
#[ignore]
async fn test_resume_interrupted_download() { }
```

**Test Requirements:**
- [ ] Small test audiobook (<10MB)
- [ ] Network throttling simulation
- [ ] Cancellation handling

#### 2.4 Database Migrations (`tests/integration/migrations.rs`)
```rust
#[tokio::test]
async fn test_migrate_from_v1_to_v2() {
    // Create v1 database
    let db = create_v1_database();

    // Apply migration
    run_migrations(&db).await.unwrap();

    // Verify schema and data integrity
    verify_v2_schema(&db).await;
}
```

**Coverage:**
- [ ] All migration steps
- [ ] Data preservation
- [ ] Rollback capability

---

### 3. End-to-End Tests (5% of test coverage)

#### 3.1 Full User Workflows (`tests/e2e/`)

**Scenario 1: New User Onboarding**
```rust
#[tokio::test]
#[ignore]
async fn test_new_user_full_flow() {
    // 1. Launch app
    // 2. Sign in to Audible
    // 3. Sync library
    // 4. Download one book
    // 5. Verify playback
}
```

**Scenario 2: Existing User Resume**
```rust
#[tokio::test]
#[ignore]
async fn test_resume_partial_download() {
    // 1. Start download
    // 2. Force kill app
    // 3. Restart app
    // 4. Verify download resumes
}
```

**Tools:**
- Android: Espresso / Appium
- iOS: XCUITest / Appium
- React Native: Detox

---

### 4. Device Testing Matrix

#### 4.1 Android Devices
| Device | OS Version | Architecture | Priority |
|--------|------------|--------------|----------|
| Pixel 6 | Android 14 | ARM64 | High |
| Samsung Galaxy S21 | Android 13 | ARM64 | High |
| Older device | Android 10 | ARM64 | Medium |
| Emulator | Android 11 | x86_64 | High |

**Test Cases:**
- [ ] Install and launch
- [ ] Sign in flow
- [ ] Library sync (1000+ books)
- [ ] Download audiobook (WiFi)
- [ ] Download audiobook (mobile data)
- [ ] Background download
- [ ] Storage permissions
- [ ] Low storage handling
- [ ] Network interruption recovery

#### 4.2 iOS Devices
| Device | OS Version | Architecture | Priority |
|--------|------------|--------------|----------|
| iPhone 14 Pro | iOS 17 | ARM64 | High |
| iPhone 12 | iOS 16 | ARM64 | High |
| iPhone SE (2nd) | iOS 15 | ARM64 | Medium |
| Simulator | iOS 17 | ARM64 | High |

**Test Cases:**
- [ ] TestFlight distribution
- [ ] Background app refresh
- [ ] iCloud sync (settings)
- [ ] Files app integration
- [ ] AirPlay compatibility

---

### 5. Performance Testing

#### 5.1 Benchmarks (`benches/`)
```rust
use criterion::{criterion_group, criterion_main, Criterion};

fn bench_decrypt_chunk(c: &mut Criterion) {
    let chunk = load_sample_chunk();
    c.bench_function("decrypt 1MB chunk", |b| {
        b.iter(|| decrypt_aax_chunk(&chunk))
    });
}

criterion_group!(benches, bench_decrypt_chunk);
criterion_main!(benches);
```

**Metrics:**
- [ ] Decryption speed (MB/s)
- [ ] Database query time
- [ ] Library sync time (per 100 books)
- [ ] Memory usage during download
- [ ] App launch time

**Tools:**
```toml
criterion = "0.5"        # Benchmarking
```

#### 5.2 Profiling
- **Android**: Android Profiler (CPU, Memory, Network)
- **iOS**: Instruments (Time Profiler, Allocations)
- **Rust**: `cargo flamegraph`

**Profile Targets:**
- [ ] Download + decrypt flow
- [ ] Database queries under load
- [ ] Memory leaks during long operations

---

### 6. Security Testing

#### 6.1 Credential Storage
- [ ] Activation bytes encrypted at rest
- [ ] OAuth tokens secured (Android KeyStore / iOS Keychain)
- [ ] No plaintext credentials in logs
- [ ] Memory zeroing for sensitive data

#### 6.2 DRM Compliance
- [ ] Activation bytes never transmitted
- [ ] Decrypted files only on device
- [ ] Cache cleanup after errors

#### 6.3 Penetration Testing
- [ ] MITM attack resistance (TLS pinning)
- [ ] SQL injection (parameterized queries)
- [ ] Path traversal (filename sanitization)

---

### 7. Regression Testing

#### 7.1 Automated Regression Suite
Run before each release:
```bash
# Unit tests
cargo test

# Integration tests (requires test account)
cargo test --test '*' -- --ignored

# E2E tests on devices
npm run test:e2e:android
npm run test:e2e:ios
```

#### 7.2 Smoke Tests (5 minutes)
Critical path verification:
1. App launches
2. User can sign in
3. Library loads
4. One audiobook downloads
5. Playback starts

---

### 8. Test Data Management

#### 8.1 Test Fixtures
```
tests/fixtures/
├── sample_books.json          # Mock library data
├── sample_aax_header.bin      # AAX file header
├── sample_aaxc_manifest.json  # AAXC manifest
├── test_account.json          # Test credentials (gitignored)
└── sample_audio_10s.aax       # 10-second test file
```

#### 8.2 Test Account Setup
- Disposable Audible account
- 2-3 cheap/free audiobooks for testing
- Store credentials in `.env.test` (gitignored)

---

### 9. Continuous Integration

#### 9.1 GitHub Actions Workflow
```yaml
name: Test Suite

on: [push, pull_request]

jobs:
  rust-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - name: Run Rust unit tests
        run: cargo test

  android-tests:
    runs-on: ubuntu-latest
    steps:
      - name: Run Android instrumented tests
        run: npm run test:android

  ios-tests:
    runs-on: macos-latest
    steps:
      - name: Run iOS tests
        run: npm run test:ios
```

#### 9.2 Pre-commit Hooks
```bash
#!/bin/sh
# .git/hooks/pre-commit
cargo test --all
cargo fmt --all -- --check
cargo clippy -- -D warnings
```

---

### 10. Test Coverage Goals

| Component | Target Coverage | Current |
|-----------|----------------|---------|
| Error types | 100% | 0% |
| HTTP client | 90% | 0% |
| Database | 85% | 0% |
| Crypto | 95% | 0% |
| File ops | 80% | 0% |
| API integration | 70% | 0% |
| **Overall** | **80%** | **0%** |

**Tools:**
```toml
tarpaulin = "0.31"       # Code coverage
```

```bash
cargo tarpaulin --out Html --output-dir coverage/
```

---

### 11. Bug Tracking & Test Cases

#### 11.1 Test Case Template
```markdown
**Test Case ID**: TC-001
**Feature**: Authentication
**Scenario**: Sign in with valid credentials
**Preconditions**: App installed, network available
**Steps**:
1. Launch app
2. Navigate to Account tab
3. Enter email and password
4. Tap "Sign In"
**Expected Result**: User logged in, library syncs
**Actual Result**:
**Status**: Pass/Fail
**Bug ID**: (if failed)
```

#### 11.2 Critical Bug Criteria
- App crashes
- Data loss
- Security vulnerability
- Unable to download/decrypt

---

### Testing Schedule

| Week | Testing Focus |
|------|---------------|
| 1-2 | Unit tests for infrastructure |
| 3-4 | Integration tests for auth & API |
| 5-7 | Crypto unit tests + integration |
| 8-10 | Download flow E2E tests |
| 11-12 | Device testing matrix |
| 13-15 | Performance + security audits |

---

---

## Migration Path from Libation

### Phase 1: Read-Only Library
1. Sync library from Audible
2. Display in React Native app
3. No downloads yet

### Phase 2: Download (No DRM)
1. Download unencrypted books only
2. Test download manager

### Phase 3: AAX Decryption
1. Implement activation bytes
2. AAX decryption
3. Full workflow

### Phase 4: AAXC (Advanced)
1. Widevine implementation
2. AAXC support

---

## Success Metrics

- [ ] Authenticate with Audible account
- [ ] Sync full library (1000+ books)
- [ ] Download and decrypt AAX audiobook
- [ ] Convert to M4B with chapters
- [ ] Playback in mobile audio player
- [ ] Download progress tracking
- [ ] Background downloads
- [ ] Handle network interruptions

---

## Timeline Summary

| Phase | Duration | Components |
|-------|----------|------------|
| 1. Infrastructure | 2 weeks | Errors, HTTP, Database |
| 2. Auth & API | 2 weeks | OAuth, Library Sync |
| 3. DRM & Crypto | 3 weeks | AAX, AAXC, Activation |
| 4. Download & Audio | 3 weeks | Download Manager, FFmpeg |
| 5. RN Integration | 2 weeks | JNI/FFI, iOS Bridge |
| 6. Advanced Features | 3 weeks | Batch, Background, PDF |
| **Total** | **15 weeks** | **~3-4 months** |

---

## Next Immediate Steps

1. **Setup Cargo workspace** with feature flags
2. **Implement error types** (start with `src/error.rs`)
3. **HTTP client** with Audible API base
4. **Database schema** ported from Libation
5. **Basic authentication** (OAuth flow)

Once these are complete, you'll have the foundation to build the rest of the features incrementally.
