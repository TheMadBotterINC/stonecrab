use std::fmt;

/// A value whose formatting never invokes the wrapped value's formatter.
///
/// This prevents accidental logging, not memory inspection. It does not claim
/// zeroization or prevent callers from deliberately logging [`Self::expose`].
/// Secrets are not cloneable by default.
#[non_exhaustive]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    /// Wrap a credential without exposing its representation to formatting.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::Secret;
    /// let credential = Secret::new("example-only");
    /// assert_eq!(format!("{credential:?}"), "[REDACTED]");
    /// assert_eq!(*credential.expose(), "example-only");
    /// ```
    #[must_use]
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Explicitly borrow the secret for authentication, never for diagnostics.
    #[must_use]
    pub const fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl<T> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_formatting_is_redacted_even_without_inner_format_traits() {
        struct Credential;
        let secret = Secret::new(Credential);
        assert_eq!(format!("{secret}"), "[REDACTED]", "display is redacted");
        assert_eq!(format!("{secret:?}"), "[REDACTED]", "debug is redacted");
        assert_eq!(
            format!("{secret:#?}"),
            "[REDACTED]",
            "pretty debug is redacted"
        );
        let text = Secret::new(String::from("example-only"));
        assert_eq!(
            text.expose(),
            "example-only",
            "explicit access preserves credential"
        );
        assert_eq!(
            format!("{text:?}"),
            "[REDACTED]",
            "text also stays redacted"
        );
    }
}
