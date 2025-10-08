# Swift Integration Guide for Rust iOS Bridge

This document explains how to call Rust functions from Swift using the C FFI bridge.

## Overview

The Rust core functionality is exposed through C-compatible FFI functions defined in `ios_bridge.h`. All functions return JSON strings that must be manually freed after use.

## Basic Pattern

Every Rust function call follows this pattern:

```swift
// 1. Call Rust function (returns char* pointer)
let resultPtr = rust_some_function(arg1, arg2)

// 2. Ensure the pointer is freed when done (even if error occurs)
defer { rust_free_string(resultPtr) }

// 3. Convert C string to Swift String
guard let jsonString = String(cString: resultPtr, encoding: .utf8) else {
    throw RustBridgeError.invalidResponse
}

// 4. Parse JSON response
guard let jsonData = jsonString.data(using: .utf8),
      let response = try? JSONSerialization.jsonObject(with: jsonData) as? [String: Any] else {
    throw RustBridgeError.jsonParseError
}

// 5. Check for success
if let success = response["success"] as? Bool, success {
    // Extract data
    if let data = response["data"] as? [String: Any] {
        // Use data
    }
} else {
    // Handle error
    let errorMessage = response["error"] as? String ?? "Unknown error"
    throw RustBridgeError.rustError(errorMessage)
}
```

## Complete Swift Wrapper Class

Here's a complete Swift wrapper class that simplifies calling Rust functions:

```swift
import Foundation

enum RustBridgeError: Error {
    case invalidResponse
    case jsonParseError
    case rustError(String)
}

struct RustResponse<T: Decodable> {
    let success: Bool
    let data: T?
    let error: String?
}

class RustBridge {

    // MARK: - Helper Methods

    /// Call a Rust function that returns a JSON string
    private static func callRust<T: Decodable>(
        _ rustFunction: () -> UnsafeMutablePointer<CChar>?
    ) throws -> T {
        guard let resultPtr = rustFunction() else {
            throw RustBridgeError.invalidResponse
        }

        defer { rust_free_string(resultPtr) }

        guard let jsonString = String(cString: resultPtr, encoding: .utf8) else {
            throw RustBridgeError.invalidResponse
        }

        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase

        let jsonData = jsonString.data(using: .utf8)!
        let response = try decoder.decode(RustResponse<T>.self, from: jsonData)

        if response.success, let data = response.data {
            return data
        } else {
            throw RustBridgeError.rustError(response.error ?? "Unknown error")
        }
    }

    // MARK: - Authentication

    struct OAuthUrlResult: Codable {
        let authorizationUrl: String
        let pkceVerifier: String
        let state: String
    }

    static func generateOAuthUrl(localeCode: String, deviceSerial: String) throws -> OAuthUrlResult {
        return try callRust {
            rust_generate_oauth_url(localeCode, deviceSerial)
        }
    }

    struct AuthCodeResult: Codable {
        let authorizationCode: String
    }

    static func parseOAuthCallback(callbackUrl: String) throws -> AuthCodeResult {
        return try callRust {
            rust_parse_oauth_callback(callbackUrl)
        }
    }

    struct TokenResult: Codable {
        let accessToken: String
        let refreshToken: String
        let expiresIn: Int
        let tokenType: String
    }

    static func exchangeAuthCode(
        localeCode: String,
        authCode: String,
        deviceSerial: String,
        pkceVerifier: String
    ) throws -> TokenResult {
        return try callRust {
            rust_exchange_auth_code(localeCode, authCode, deviceSerial, pkceVerifier)
        }
    }

    static func refreshAccessToken(
        localeCode: String,
        refreshToken: String,
        deviceSerial: String
    ) throws -> TokenResult {
        return try callRust {
            rust_refresh_access_token(localeCode, refreshToken, deviceSerial)
        }
    }

    struct ActivationBytesResult: Codable {
        let activationBytes: String
    }

    static func getActivationBytes(
        localeCode: String,
        accessToken: String
    ) throws -> ActivationBytesResult {
        return try callRust {
            rust_get_activation_bytes(localeCode, accessToken)
        }
    }

    // MARK: - Database

    struct InitDatabaseResult: Codable {
        let initialized: Bool
    }

    static func initDatabase(dbPath: String) throws -> InitDatabaseResult {
        return try callRust {
            rust_init_database(dbPath)
        }
    }

    struct SyncLibraryResult: Codable {
        let totalItems: Int
        let booksAdded: Int
        let booksUpdated: Int
        let booksAbsent: Int
        let errors: [String]
    }

    static func syncLibrary(dbPath: String, accountJson: String) throws -> SyncLibraryResult {
        return try callRust {
            rust_sync_library(dbPath, accountJson)
        }
    }

    struct Book: Codable {
        let asin: String
        let title: String
        let subtitle: String?
        let authors: [String]
        let narrators: [String]
        let seriesTitle: String?
        let seriesSequence: String?
        let runtimeLengthMin: Int?
        let releaseDate: String?
        let language: String?
        let publisherName: String?
        let formatType: String?
        let purchaseDate: String?
        let isDownloaded: Bool
        let downloadStatus: String?
    }

    struct GetBooksResult: Codable {
        let books: [Book]
        let totalCount: Int
    }

    static func getBooks(dbPath: String, offset: Int64, limit: Int64) throws -> GetBooksResult {
        return try callRust {
            rust_get_books(dbPath, offset, limit)
        }
    }

    struct SearchBooksResult: Codable {
        let books: [Book]
    }

    static func searchBooks(dbPath: String, query: String) throws -> SearchBooksResult {
        return try callRust {
            rust_search_books(dbPath, query)
        }
    }

    // MARK: - Download/Decrypt

    struct DownloadBookResult: Codable {
        let bytesDownloaded: UInt64
        let outputPath: String
    }

    static func downloadBook(
        asin: String,
        accessToken: String,
        localeCode: String,
        outputPath: String
    ) throws -> DownloadBookResult {
        return try callRust {
            rust_download_book(asin, accessToken, localeCode, outputPath)
        }
    }

    struct DecryptAaxResult: Codable {
        let outputPath: String
        let fileSize: UInt64
    }

    static func decryptAax(
        inputPath: String,
        outputPath: String,
        activationBytes: String
    ) throws -> DecryptAaxResult {
        return try callRust {
            rust_decrypt_aax(inputPath, outputPath, activationBytes)
        }
    }

    // MARK: - Utilities

    struct ValidateActivationBytesResult: Codable {
        let valid: Bool
    }

    static func validateActivationBytes(activationBytes: String) throws -> ValidateActivationBytesResult {
        return try callRust {
            rust_validate_activation_bytes(activationBytes)
        }
    }

    struct Locale: Codable {
        let countryCode: String
        let name: String
        let domain: String
    }

    struct GetSupportedLocalesResult: Codable {
        let locales: [Locale]
    }

    static func getSupportedLocales() throws -> GetSupportedLocalesResult {
        return try callRust {
            rust_get_supported_locales()
        }
    }
}
```

## Usage Examples

### Example 1: OAuth Flow

```swift
import Foundation

func startOAuthFlow() async throws {
    // Step 1: Generate OAuth URL
    let oauthResult = try RustBridge.generateOAuthUrl(
        localeCode: "us",
        deviceSerial: "1234567890abcdef1234567890abcdef"
    )

    print("Visit URL: \(oauthResult.authorizationUrl)")

    // Store pkceVerifier for later
    UserDefaults.standard.set(oauthResult.pkceVerifier, forKey: "pkce_verifier")

    // Step 2: Wait for callback (in your web view delegate)
    // let callbackUrl = "https://localhost/callback?code=ABC123..."

    // Step 3: Parse callback
    let authCodeResult = try RustBridge.parseOAuthCallback(
        callbackUrl: callbackUrl
    )

    // Step 4: Exchange for tokens
    let pkceVerifier = UserDefaults.standard.string(forKey: "pkce_verifier")!
    let tokens = try RustBridge.exchangeAuthCode(
        localeCode: "us",
        authCode: authCodeResult.authorizationCode,
        deviceSerial: "1234567890abcdef1234567890abcdef",
        pkceVerifier: pkceVerifier
    )

    // Save tokens
    UserDefaults.standard.set(tokens.accessToken, forKey: "access_token")
    UserDefaults.standard.set(tokens.refreshToken, forKey: "refresh_token")
}
```

### Example 2: Fetch and Display Books

```swift
import Foundation

func fetchBooks() async throws {
    let dbPath = FileManager.default
        .urls(for: .documentDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("libation.db")
        .path

    // Initialize database
    _ = try RustBridge.initDatabase(dbPath: dbPath)

    // Get books with pagination
    let result = try RustBridge.getBooks(
        dbPath: dbPath,
        offset: 0,
        limit: 50
    )

    print("Total books: \(result.totalCount)")

    for book in result.books {
        print("\(book.title) by \(book.authors.joined(separator: ", "))")
    }
}
```

### Example 3: Download and Decrypt

```swift
import Foundation

func downloadAndDecrypt(asin: String) async throws {
    let accessToken = UserDefaults.standard.string(forKey: "access_token")!

    let documentsPath = FileManager.default
        .urls(for: .documentDirectory, in: .userDomainMask)[0]

    let aaxPath = documentsPath.appendingPathComponent("\(asin).aax").path
    let m4bPath = documentsPath.appendingPathComponent("\(asin).m4b").path

    // Download
    let downloadResult = try RustBridge.downloadBook(
        asin: asin,
        accessToken: accessToken,
        localeCode: "us",
        outputPath: aaxPath
    )

    print("Downloaded \(downloadResult.bytesDownloaded) bytes")

    // Get activation bytes
    let activationResult = try RustBridge.getActivationBytes(
        localeCode: "us",
        accessToken: accessToken
    )

    // Decrypt
    let decryptResult = try RustBridge.decryptAax(
        inputPath: aaxPath,
        outputPath: m4bPath,
        activationBytes: activationResult.activationBytes
    )

    print("Decrypted to \(decryptResult.outputPath)")
}
```

### Example 4: Error Handling

```swift
import Foundation

func safeRustCall() {
    do {
        let locales = try RustBridge.getSupportedLocales()
        print("Found \(locales.locales.count) locales")
    } catch RustBridgeError.rustError(let message) {
        print("Rust error: \(message)")
    } catch RustBridgeError.invalidResponse {
        print("Invalid response from Rust")
    } catch RustBridgeError.jsonParseError {
        print("Failed to parse JSON")
    } catch {
        print("Unexpected error: \(error)")
    }
}
```

## Expo Module Integration

To use this in an Expo module, create a Swift module that wraps the Rust bridge:

```swift
// ExpoRustBridgeModule.swift
import ExpoModulesCore

public class ExpoRustBridgeModule: Module {
    public func definition() -> ModuleDefinition {
        Name("ExpoRustBridge")

        AsyncFunction("generateOAuthUrl") { (localeCode: String, deviceSerial: String) -> [String: Any] in
            let result = try RustBridge.generateOAuthUrl(
                localeCode: localeCode,
                deviceSerial: deviceSerial
            )
            return [
                "authorizationUrl": result.authorizationUrl,
                "pkceVerifier": result.pkceVerifier,
                "state": result.state
            ]
        }

        AsyncFunction("getBooks") { (dbPath: String, offset: Int, limit: Int) -> [String: Any] in
            let result = try RustBridge.getBooks(
                dbPath: dbPath,
                offset: Int64(offset),
                limit: Int64(limit)
            )

            let booksJson = result.books.map { book -> [String: Any] in
                return [
                    "asin": book.asin,
                    "title": book.title,
                    "authors": book.authors,
                    // ... other fields
                ]
            }

            return [
                "books": booksJson,
                "totalCount": result.totalCount
            ]
        }

        // Add more functions as needed...
    }
}
```

## Memory Safety Notes

1. **Always use `defer`** to ensure `rust_free_string()` is called even if an error occurs
2. **Never free the same pointer twice** - this will cause a crash
3. **Always free pointers** - not freeing causes memory leaks
4. **Don't use pointers after freeing** - they become invalid

## Thread Safety

The Rust bridge uses a Tokio runtime for async operations. All functions are thread-safe and can be called from any thread. However, for best performance, avoid calling blocking Rust functions on the main thread in your app.

```swift
Task.detached {
    let books = try await RustBridge.getBooks(dbPath: dbPath, offset: 0, limit: 50)
    await MainActor.run {
        // Update UI on main thread
        self.updateUI(with: books)
    }
}
```
