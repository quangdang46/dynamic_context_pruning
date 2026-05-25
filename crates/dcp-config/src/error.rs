//! `ConfigError` — every failure surface of the configuration crate.

use std::path::PathBuf;

use thiserror::Error;

/// Diagnostic returned by [`crate::load_default`] and
/// [`crate::Config::validate`].
///
/// Marked `#[non_exhaustive]` so new variants can be introduced without
/// breaking downstream `match` arms.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// I/O failure while reading a config file.
    #[error("could not read config file `{path}`: {source}")]
    Io {
        /// Path that failed to read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// JSONC / JSON5 parse failure.
    #[error("could not parse config file `{path}`: {message}")]
    Parse {
        /// Path that failed to parse.
        path: PathBuf,
        /// Parser diagnostic.
        message: String,
    },
    /// JSON deserialisation failed after merge.
    #[error("could not deserialise merged config: {0}")]
    Deserialize(String),
    /// Validation rule violated (SPEC.md §10.3).
    #[error("invalid configuration: {0}")]
    Validation(String),
    /// A protected file glob did not compile.
    #[error("invalid glob `{pattern}`: {message}")]
    InvalidGlob {
        /// Offending glob pattern.
        pattern: String,
        /// Compiler diagnostic.
        message: String,
    },
}

impl ConfigError {
    /// Convenience constructor for [`ConfigError::Validation`].
    pub fn validation(msg: impl Into<String>) -> Self {
        Self::Validation(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_constructor_smoke() {
        let e = ConfigError::validation("bad");
        assert!(matches!(e, ConfigError::Validation(_)));
        assert_eq!(format!("{e}"), "invalid configuration: bad");
    }
}
