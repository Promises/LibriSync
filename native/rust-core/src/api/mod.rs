// LibriSync - Audible Library Sync for Mobile
// Copyright (C) 2025 Henning Berge
//
// This program is a Rust port of Libation (https://github.com/rmcrackan/Libation)
// Original work Copyright (C) Libation contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.


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
pub mod signing;

// Re-export commonly used types
pub use auth::{Account, Identity};
pub use client::{AudibleClient, AudibleDomain, ClientConfig};
pub use library::LibraryOptions;
pub use registration::{RegistrationResponse, RegistrationData};
pub use customer::CustomerInformation;
