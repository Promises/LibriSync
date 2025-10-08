# Agent Implementation Plan - Phase 1: Infrastructure

> **📊 STATUS (Oct 8, 2025): Phase 1 COMPLETE ✅**
> - All 113 Rust unit tests passing
> - OAuth authentication working end-to-end
> - Paginated library sync implemented with progressive UI updates
> - Full bridge layer complete (JNI for Android, C FFI for iOS)
> - See `PROGRESS.md` for current status and next steps

## Overview

This document outlines how to use the general-purpose agent to implement Phase 1 (Core Infrastructure) of the **direct Libation → Rust library port**.

**Critical Context for Agent:**
- This is a **direct 1:1 port** of Libation's C# codebase to Rust
- NOT a reimagining or rewrite - translate Libation's logic directly
- Each Rust module must correspond to a Libation C# component
- Reference the original C# source in `references/Libation/Source/` for all logic
- Maintain Libation's architecture, data models, and method signatures (adapted to Rust idioms)

The agent will help with:
1. Setting up the Rust crate structure (matching Libation's component structure)
2. Porting Libation's error handling patterns to Rust
3. Creating HTTP client based on Libation's ApiExtended.cs
4. Porting Entity Framework data models to Rust
5. Writing unit tests matching Libation's test suite

---

## Agent Task Breakdown

### Task 1: Setup Rust Crate Structure

**Agent Prompt:**
```
IMPORTANT: This task sets up the Rust library structure to directly mirror Libation's C# component architecture.

Set up the Rust crate structure for native/rust-core/ matching Libation's architecture in references/Libation/Source/.

Requirements:
1. Create directory structure mirroring Libation components:
   - src/api/ (mod.rs, auth.rs, client.rs, library.rs, content.rs, license.rs) → AudibleUtilities/
   - src/crypto/ (mod.rs, activation.rs, aax.rs, aaxc.rs, widevine.rs) → AaxDecrypter/
   - src/download/ (mod.rs, manager.rs, stream.rs, progress.rs) → FileLiberator/
   - src/audio/ (mod.rs, decoder.rs, converter.rs, metadata.rs) → FileLiberator/
   - src/storage/ (mod.rs, database.rs, models.rs, migrations.rs, queries.rs) → DataLayer/
   - src/file/ (mod.rs, manager.rs, paths.rs) → FileManager/
   - src/error.rs

2. Update Cargo.toml with Phase 1 dependencies from LIBATION_PORT_PLAN.md:
   - thiserror, anyhow (errors)
   - reqwest, tokio (HTTP)
   - sqlx (database)
   - serde, serde_json (serialization)

3. In each placeholder mod.rs, add header:
   ```rust
   //! Direct port of Libation's [Component] functionality
   //! Reference: references/Libation/Source/[Component]/
   //!
   //! TODO: Port functionality from C# source
   ```

4. Update src/lib.rs:
   - Add module declarations for all new modules
   - Add doc comment explaining this is a direct Libation library port
   - Keep existing jni_bridge and log_from_rust unchanged

5. Do not modify jni_bridge.rs or log_from_rust.

Validate with: cargo check
```

**Expected Output:**
- Complete directory structure
- Updated Cargo.toml with Phase 1 dependencies
- Module declarations in lib.rs
- Placeholder files with module structure

**Validation:**
```bash
cargo check
```

---

### Task 2: Implement Error Types

**Agent Prompt:**
```
Implement comprehensive error types in native/rust-core/src/error.rs for the RN Audible project.

Requirements:
1. Use thiserror crate for error derivation
2. Create error enums for:
   - ApiError (HTTP status codes, network errors, auth failures)
   - CryptoError (decryption failures, key errors, activation bytes errors)
   - StorageError (database errors, file system errors)
   - DownloadError (network interruptions, resume failures)
   - AudioError (conversion failures, metadata errors)

3. Each error should:
   - Have descriptive variants
   - Include context (file paths, ASINs, status codes where applicable)
   - Implement Display with user-friendly messages
   - Support conversion with #[from] for underlying errors

4. Create a Result type alias: `pub type Result<T> = std::result::Result<T, Error>;`

5. Write unit tests for:
   - Error construction
   - Error message formatting
   - Error conversion chains
   - Serialization (for passing errors to JS)

Reference the error handling patterns from anyhow and thiserror documentation.
```

**Expected Output:**
- `src/error.rs` with comprehensive error types
- Unit tests in the same file
- Clear documentation comments

**Validation:**
```bash
cargo test error
cargo doc --no-deps --open
```

---

### Task 3: Implement HTTP Client

**Agent Prompt:**
```
IMPORTANT: This is a DIRECT PORT of Libation's HTTP client to Rust. Base the implementation on Libation's ApiExtended.cs.

Implement an async HTTP client in native/rust-core/src/api/client.rs as a port of Libation's Audible API client.

Requirements:
1. **Read references/Libation/Source/AudibleUtilities/ApiExtended.cs** to understand:
   - How Libation structures API calls
   - Retry logic implementation
   - Cookie/session management
   - Error handling patterns

2. Create ApiClient struct matching Libation's patterns:
   - Base URL configuration (https://api.audible.com)
   - Cookie jar for session management (like Libation)
   - Custom User-Agent header matching Libation's
   - Timeout configuration (match Libation's defaults)
   - Retry logic with exponential backoff (same retry count as Libation)

3. Port Libation's HTTP methods to Rust equivalents:
   - Constructor matching Libation's initialization
   - GET/POST methods with same parameter patterns
   - Streaming download support (for audio files)
   - JSON deserialization matching Libation's DTOs

4. Replicate Libation's features:
   - Automatic retry on 5xx errors and network failures (same logic)
   - Progress tracking callbacks (same pattern as Libation)
   - Custom headers per-request
   - Cookie persistence across requests

5. Write unit tests using mockito:
   - Successful requests
   - Retry logic (mock 500 → 500 → 200)
   - Timeout handling
   - Cookie persistence across requests

6. Add header comment:
   ```rust
   //! Direct port of Libation's Audible API HTTP client
   //! Reference: references/Libation/Source/AudibleUtilities/ApiExtended.cs
   ```

Study the C# implementation BEFORE writing Rust code. This is a translation, not a new design.
```

**Expected Output:**
- `src/api/client.rs` with ApiClient implementation
- `src/api/mod.rs` with module exports
- Unit tests with mocked HTTP responses
- Documentation with usage examples

**Validation:**
```bash
cargo test api::client
```

---

### Task 4: Implement Database Layer - Models

**Agent Prompt:**
```
IMPORTANT: This is a DIRECT PORT of Libation's Entity Framework data models to Rust. Do NOT design new models - translate the existing C# models exactly.

Implement database models in native/rust-core/src/storage/models.rs as a direct port of Libation's Entity Framework schema.

Requirements:
1. **Read the C# source files** from references/Libation/Source/DataLayer/EfClasses/:
   - Book.cs
   - LibraryBook.cs
   - UserDefinedItem.cs
   - Series.cs
   - Contributor.cs
   - Category.cs
   - SeriesBook.cs
   - BookContributor.cs
   - BookCategory.cs

2. **Create exact Rust equivalents** for each C# class:
   - Book: Port ALL properties (ASIN, title, authors array, narrators array, runtime_length_min, description, picture_id, etc.)
   - LibraryBook: Port relationship to Book, account info, locale
   - UserDefinedItem: Port tags, download status, last downloaded metadata
   - Series: Port name, ASIN, relationship to books
   - Contributor: Port name, ASIN, role
   - Category: Port name, hierarchy

3. **Preserve exact field names** from C# (converted to snake_case):
   - C#: `RuntimeLengthMinutes` → Rust: `runtime_length_min`
   - C#: `PictureId` → Rust: `picture_id`
   - This ensures potential database compatibility

4. Each struct must:
   - Use serde for JSON serialization
   - Have sqlx derives for database mapping
   - Include all relationships (foreign keys)
   - Add doc comments referencing the C# source

5. Port enums from Libation:
   - LiberatedStatus (from Libation enum)
   - AudioFormat (from Libation AudioFormat.cs)

6. Add header comment:
   ```rust
   //! Direct port of Libation's Entity Framework data models
   //! Reference: references/Libation/Source/DataLayer/EfClasses/
   ```

This is a TRANSLATION task, not a design task. Match the C# schema exactly.
```

**Expected Output:**
- `src/storage/models.rs` with all entity models
- Enums for status and format types
- serde and sqlx derives
- Documentation

**Validation:**
```bash
cargo check
cargo doc --no-deps --open
```

---

### Task 5: Implement Database Layer - Schema & Migrations

**Agent Prompt:**
```
IMPORTANT: This is a DIRECT PORT of Libation's Entity Framework database schema to SQLite with sqlx. Use Libation's schema as the authoritative reference.

Implement the SQLite database schema in native/rust-core/src/storage/ as a direct port of Libation's database.

Requirements:
1. **Study Libation's database structure**:
   - Read references/Libation/Source/DataLayer/LibationContext.cs (DbContext configuration)
   - Read references/Libation/Source/DataLayer/Configurations/ (table configurations)
   - Read references/Libation/Source/DataLayer/Migrations/ (migration history)
   - Note the EXACT table structure, column names, types, and relationships

2. In src/storage/database.rs:
   - Create Database struct with SqlitePool (equivalent to LibationContext)
   - Implement `Database::new(path)` matching Libation's database initialization
   - Implement connection pooling
   - Add migration runner

3. In src/storage/migrations.rs:
   - Port the EXACT schema from Libation's latest migration
   - Create SQL for tables (use same table/column names as Entity Framework):
     - Books table (all columns from Book.cs + Configurations/BookConfig.cs)
     - LibraryBooks table (from LibraryBook.cs + LibraryBookConfig.cs)
     - UserDefinedItems table (from UserDefinedItem.cs)
     - Series, Contributors, Categories tables
     - Junction tables (BookContributors, BookCategories, SeriesBooks)
   - Include all indexes from Libation
   - Include all foreign key constraints from Libation
   - Implement versioned migrations

4. Use sqlx for compile-time query checking

5. Write integration tests:
   - Create database in temp directory
   - Run migrations
   - Verify schema matches Libation's
   - Test foreign key constraints

6. Add header comment:
   ```rust
   //! Direct port of Libation's Entity Framework database schema
   //! Reference: references/Libation/Source/DataLayer/
   ```

**Critical:** The goal is database compatibility with Libation where possible. Use the same table/column names, types, and relationships.
```

**Expected Output:**
- `src/storage/database.rs` with Database struct
- `src/storage/migrations.rs` with SQL migrations
- Integration tests that create and migrate database

**Validation:**
```bash
cargo test storage::database
cargo test storage::migrations
```

---

### Task 6: Implement Database Layer - Queries

**Agent Prompt:**
```
IMPORTANT: Port database query patterns from Libation to Rust. Use references/Libation/Source/DataLayer/QueryObjects/ as the reference.

Implement database query helpers in native/rust-core/src/storage/queries.rs as a direct port of Libation's query patterns.

Requirements:
1. **Study Libation's query objects** in references/Libation/Source/DataLayer/QueryObjects/

2. Create query functions matching Libation's common operations:

Books:
- `insert_book(db, book) -> Result<()>`
- `get_book_by_asin(db, asin) -> Result<Book>`
- `get_all_books(db) -> Result<Vec<Book>>`
- `update_book(db, book) -> Result<()>`
- `delete_book(db, asin) -> Result<()>`
- `search_books(db, query) -> Result<Vec<Book>>`

LibraryBooks:
- `insert_library_book(db, library_book) -> Result<()>`
- `get_library_books_for_account(db, account) -> Result<Vec<LibraryBook>>`
- `update_download_status(db, asin, status) -> Result<()>`

Series:
- `get_books_in_series(db, series_asin) -> Result<Vec<Book>>`

Complex queries:
- `get_books_by_author(db, author_name) -> Result<Vec<Book>>`
- `get_books_by_narrator(db, narrator_name) -> Result<Vec<Book>>`
- `get_books_with_status(db, status) -> Result<Vec<Book>>`

2. Use sqlx query macros for type safety
3. Implement pagination for large result sets
4. Add transaction support for multi-step operations

5. Write unit tests with in-memory database:
   - Insert and retrieve books
   - Update operations
   - Complex queries with joins
   - Transaction rollback

Reference query patterns from references/Libation/Source/DataLayer/QueryObjects/
```

**Expected Output:**
- `src/storage/queries.rs` with CRUD and search functions
- Unit tests with test fixtures
- Documentation with usage examples

**Validation:**
```bash
cargo test storage::queries
```

---

### Task 7: Integrate Infrastructure Components

**Agent Prompt:**
```
Update native/rust-core/src/lib.rs to integrate the new infrastructure components (error types, HTTP client, database) and create a unified API surface.

Requirements:
1. Export public APIs:
   - Error types
   - ApiClient
   - Database
   - Models

2. Create high-level initialization function:
   - `pub fn initialize(config: Config) -> Result<AppContext>`
   - AppContext should hold: database connection, API client, config

3. Update existing log_from_rust function to use new error types

4. Add new JNI bridge functions for testing infrastructure:
   - `test_database_connection() -> Result<String>`
   - `test_api_connection() -> Result<String>`

5. Write integration tests in tests/integration/infrastructure.rs:
   - Initialize app context
   - Verify database accessible
   - Verify API client configured

6. Update documentation in lib.rs with architecture overview
```

**Expected Output:**
- Updated lib.rs with public API surface
- AppContext struct for dependency injection
- Integration test showing all components working together

**Validation:**
```bash
cargo test
cargo doc --no-deps --open
npm run build:rust:android
```

---

## Agent Execution Plan

### Week 1: Foundation

**Day 1-2: Structure & Errors**
1. Run Task 1 (agent: setup crate structure)
2. Run Task 2 (agent: implement error types)
3. Manual review and adjust
4. Commit: "feat: add Rust project structure and error types"

**Day 3-4: HTTP Client**
1. Run Task 3 (agent: implement HTTP client)
2. Manual testing with httpbin.org
3. Commit: "feat: add HTTP client with retry logic"

**Day 5: Database Models**
1. Run Task 4 (agent: implement database models)
2. Compare with Libation C# models
3. Commit: "feat: add database models"

### Week 2: Database & Integration

**Day 1-2: Database Schema**
1. Run Task 5 (agent: implement database and migrations)
2. Test migrations manually
3. Commit: "feat: add SQLite database with migrations"

**Day 3-4: Database Queries**
1. Run Task 6 (agent: implement query helpers)
2. Run tests and verify coverage
3. Commit: "feat: add database query layer"

**Day 5: Integration**
1. Run Task 7 (agent: integrate components)
2. Run full test suite
3. Build for Android and test on device
4. Commit: "feat: integrate infrastructure components"

---

## Manual Review Checklist

After each agent task, review:

- [ ] Code compiles (`cargo check`)
- [ ] Tests pass (`cargo test`)
- [ ] Documentation builds (`cargo doc`)
- [ ] No compiler warnings
- [ ] Code follows Rust best practices
- [ ] Error messages are user-friendly
- [ ] No unwrap() or panic!() in production code
- [ ] Proper async/await usage
- [ ] Database migrations are reversible
- [ ] Sensitive data not logged

---

## Agent Invocation Examples

### Using the Task Tool

```typescript
// Task 1: Setup structure
await task({
  description: "Setup Rust crate structure",
  prompt: `[Task 1 prompt from above]`,
  subagent_type: "general-purpose"
});

// Task 2: Error types
await task({
  description: "Implement error types",
  prompt: `[Task 2 prompt from above]`,
  subagent_type: "general-purpose"
});

// etc...
```

---

## Success Criteria for Phase 1

Phase 1 is complete when:

- [x] All Cargo.toml dependencies compile ✅
- [x] Error types are comprehensive and tested ✅ (58 error variants)
- [x] HTTP client can make requests (tested with mocks) ✅
- [x] Database schema matches Libation ✅ (11 tables, 17 indexes)
- [x] Database queries work (tested) ✅
- [x] All unit tests pass (>80% coverage) ✅ (113/113 passing - 100%)
- [x] Integration test passes ✅
- [x] Rust library builds for Android ✅
- [x] Documentation is complete ✅
- [x] Code review approved ✅

**Status:** ✅ **COMPLETE** (Completed ahead of schedule)
**Actual Time:** ~10 days (vs estimated 2 weeks)

---

## Troubleshooting

### Agent doesn't follow instructions
- Break task into smaller sub-tasks
- Provide more specific examples
- Reference specific files from Libation

### Tests are failing
- Review test mocks and fixtures
- Check async/await usage
- Verify database migrations

### Android build fails
- Check JNI function signatures
- Verify no `std::io` in mobile targets
- Use `#[cfg(target_os = "android")]` conditionally

---

## Next Phase Preview

After Phase 1 is complete, we'll use agents for Phase 2 (Authentication & API):
- OAuth 2.0 flow implementation
- Device registration
- Token management
- Library sync

The foundation from Phase 1 will make Phase 2 much faster to implement.
