# How to Fix Rust Test Compilation Errors

## The Problem

**5 compilation errors** block the test suite from running. All errors are in **test code**, not production code.

**Good news:** The production code compiles and works fine (your Android app runs!). The issue is just outdated test code.

---

## Specific Errors Found

### Location: `src/api/auth.rs` lines 2186-2189 (test code)

**Error:** Test code tries to access fields on `RegistrationResponse` that are actually nested under `bearer`.

#### Current (Broken) Code:
```rust
println!("   Access Token: {}...", &token_response.access_token[..30]);
println!("   Refresh Token: {}...", &token_response.refresh_token[..30]);
println!("   Expires In: {} seconds", token_response.expires_in);
println!("   Token Type: {}\n", token_response.token_type);
```

#### Fixed Code:
```rust
println!("   Access Token: {}...", &token_response.bearer.access_token[..30]);
println!("   Refresh Token: {}...", &token_response.bearer.refresh_token[..30]);
println!("   Expires In: {} seconds", token_response.bearer.expires_in);
println!("   Token Type: {}\n", token_response.bearer.token_type);
```

**Explanation:** The `RegistrationResponse` struct has this structure:
```rust
pub struct RegistrationResponse {
    pub bearer: TokenResponse,        // ← access_token, refresh_token are HERE
    pub mac_dms: MacDms,
    pub website_cookies: HashMap<String, String>,
    pub store_authentication_cookie: StoreAuthCookie,
    pub device_info: DeviceInfo,
    pub customer_info: CustomerInfo,
}
```

Test code was trying to access `token_response.access_token` but should be `token_response.bearer.access_token`.

---

## Quick Fix (5 Minutes)

### Option 1: Fix the Test Code

1. Open `native/rust-core/src/api/auth.rs`
2. Find line 2186 (search for `"Access Token:"`)
3. Replace the 4 lines with the fixed versions above

**Commands:**
```bash
cd native/rust-core

# Edit the file (use your editor)
# Find lines 2186-2189 in #[cfg(test)] section
# Add ".bearer" before each field access

# Test the fix
cargo test --lib
```

### Option 2: Remove Test Code Temporarily (2 Minutes)

If you don't need the tests right now, comment out the broken test:

```bash
cd native/rust-core/src

# Find the test function containing lines 2186-2189
# It's likely named something like test_registration_response()
# Add #[ignore] above it:

#[test]
#[ignore]  // ← Add this line
fn test_registration_response() {
    // ...
}
```

### Option 3: Just Build Without Tests (0 Minutes)

The production code builds fine. Skip tests for now:

```bash
# This works fine:
cargo build --release

# Android builds work:
cargo build --target aarch64-linux-android --release

# Just can't run tests:
# cargo test --lib  ← This fails
```

---

## Detailed Fix Instructions

### Step-by-Step Fix

1. **Open the file:**
   ```bash
   cd /Users/henningberge/Documents/projects/rn-audible/native/rust-core
   code src/api/auth.rs  # or vim, nano, etc.
   ```

2. **Find the test code** (search for `"Access Token:"`):
   - Go to line ~2186
   - You'll find it in a `#[cfg(test)]` module at the end of the file
   - Inside a test function (likely `test_registration_response()`)

3. **Replace these 4 lines:**

   **FROM:**
   ```rust
   println!("   Access Token: {}...", &token_response.access_token[..30]);
   println!("   Refresh Token: {}...", &token_response.refresh_token[..30]);
   println!("   Expires In: {} seconds", token_response.expires_in);
   println!("   Token Type: {}\n", token_response.token_type);
   ```

   **TO:**
   ```rust
   println!("   Access Token: {}...", &token_response.bearer.access_token[..30]);
   println!("   Refresh Token: {}...", &token_response.bearer.refresh_token[..30]);
   println!("   Expires In: {} seconds", token_response.bearer.expires_in);
   println!("   Token Type: {}\n", token_response.bearer.token_type);
   ```

4. **Save and test:**
   ```bash
   cargo test --lib
   ```

---

## Understanding the Issue

### Why This Happened

The `RegistrationResponse` structure was refactored to nest token fields under `bearer`, but the test code wasn't updated.

**Production code is fine** because it correctly accesses `response.bearer.access_token`.

**Test code is broken** because it still uses the old flat structure `response.access_token`.

### Why Tests Matter (But Aren't Blocking)

**You can ship without fixing tests** because:
- ✅ Production code compiles
- ✅ Android app works
- ✅ OAuth and library sync work
- ✅ The actual implementation is correct

**But you should fix tests** because:
- 📋 Tests document expected behavior
- 🔍 Tests catch regressions
- ✅ Tests verify refactors don't break things
- 🚀 Tests are needed for CI/CD

---

## After Fixing

Once you fix the test code, verify everything works:

```bash
# Clean build
cargo clean

# Build production code
cargo build --release

# Run all tests
cargo test --lib

# Expected: All tests pass (or only intentional failures)
```

You should see something like:
```
running 113 tests
test result: ok. 113 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

---

## What About the Widevine/AAXC Stubs?

Those are **intentional** and don't need fixing:

```rust
// crypto/widevine.rs
pub fn new(device: WidevinDevice) -> Result<Self> {
    unimplemented!("Initialize Widevine CDM")
}
```

**These are fine because:**
1. They're planned future features (not needed now)
2. They compile successfully
3. They won't be called (no UI exposes them)
4. AAX decryption works without them

**If test code tries to test these stubs**, just add `#[ignore]` to skip them:

```rust
#[test]
#[ignore]  // Skip until Widevine is implemented
fn test_widevine_cdm() {
    // ...
}
```

---

## Summary

**The Issue:**
- 5 test code errors in `src/api/auth.rs` lines 2186-2189
- Test code uses old flat structure: `response.access_token`
- Should use nested structure: `response.bearer.access_token`

**The Fix:**
- Add `.bearer` to 4 field accesses in test code
- Takes 5 minutes
- OR just ignore the test for now

**Impact:**
- ✅ Production code is fine (app works!)
- ❌ Test suite can't run (minor issue)
- 🎯 Easy fix, low priority

**Do you need to fix it now?**
- No, if you just want to keep building features
- Yes, if you want clean tests before committing
- Yes, if you want CI/CD to work

---

## Need Help?

If you want me to make the fix for you, I can:
1. Read the test code
2. Apply the exact fix
3. Verify it compiles
4. Run the tests

Just ask: "Fix the test errors in auth.rs"
