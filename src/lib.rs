#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

//! # Stonecrab
//!
//! **Apache PLC4X, carcinized.** An independent, native Rust port of Apache PLC4X.
//!
//! This crate is currently in the specification stage. See the repository for details.

/// Placeholder module - implementation coming soon
pub mod placeholder {
    /// Returns the project status
    pub fn status() -> &'static str {
        "specification stage - see README for details"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status() {
        assert_eq!(placeholder::status(), "specification stage - see README for details");
    }
}
