//! Audible API client implementation
//!
//! This module ports the Audible API functionality from Libation's C# codebase.
//! It provides authentication, library management, and content access.
//!
//! # Reference C# Sources
//! - `AudibleUtilities/ApiExtended.cs` - Main API wrapper with retry logic and concurrency
//! - `AudibleUtilities/AudibleApiStorage.cs` - Token and settings storage
//! - External dependency: AudibleApi NuGet package (see Libation references)

pub mod auth;
pub mod client;
pub mod library;
pub mod content;
pub mod license;
pub mod registration;
pub mod customer;

// Re-export commonly used types
pub use auth::{Account, Identity};
pub use client::{AudibleClient, AudibleDomain, ClientConfig};
pub use library::LibraryOptions;
pub use registration::{RegistrationResponse, RegistrationData};
pub use customer::CustomerInformation;
