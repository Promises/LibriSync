//! File management and path utilities
//!
//! This module handles file operations, path generation, and naming templates.
//!
//! # Reference C# Sources
//! - `FileManager/` - File utilities and operations
//! - `LibationFileManager/` - Libation-specific file operations
//! - `FileManager/NamingTemplate/` - Template system for file naming

pub mod manager;
pub mod paths;

// Re-export commonly used types
pub use manager::FileManager;
pub use paths::PathBuilder;
