//! Thread-safety contract of the public types (section 9.11 of the coding standards).
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab::codec::{BoundedReader, BoundedWriter, ParseError};

    #[test]
    fn public_types_are_send_sync_and_static() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<BoundedReader<'static>>();
        assert_send_sync::<BoundedWriter>();
        assert_send_sync::<ParseError>();
    }
}
