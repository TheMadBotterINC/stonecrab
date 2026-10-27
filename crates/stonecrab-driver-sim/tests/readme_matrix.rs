//! The crate README's support table agrees with `SimDriver::capabilities()`.
//!
//! The table is the crate's public statement of what is supported and at which level
//! (SPEC.md 'Hardware evidence'). A capability the driver advertises must not say `planned`,
//! and one it does not advertise must not claim a level. S31 reuses this test for the
//! other driver crates.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab_core::{Driver, SubscriptionMode};
    use stonecrab_driver_sim::SimDriver;

    /// Every label a support-level cell may use in a driver README.
    const LEVELS: [&str; 6] = [
        "planned",
        "not supported",
        "build-tested",
        "simulator-tested",
        "interoperability-tested",
        "hardware-verified",
    ];

    /// `(capability label, level)` for every row of the README's support table, the table
    /// whose header is `| Capability | Support level |`. Backticks are removed from labels.
    fn support_rows() -> Vec<(String, String)> {
        include_str!("../README.md")
            .lines()
            .skip_while(|line| line.trim() != "| Capability | Support level |")
            .skip(2)
            .take_while(|line| line.starts_with('|'))
            .map(|line| {
                let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
                let [label, level] = cells.as_slice() else {
                    panic!("a support row has two cells: {line}");
                };
                (label.replace('`', ""), (*level).to_owned())
            })
            .collect()
    }

    fn level_of(label_prefix: &str) -> String {
        let rows = support_rows();
        let mut found = rows
            .iter()
            .filter(|(label, _)| label.starts_with(label_prefix));
        let (_, level) = found
            .next()
            .expect("the README has a row for this capability");
        assert!(
            found.next().is_none(),
            "exactly one row starts with {label_prefix:?}"
        );
        level.clone()
    }

    // covers: ALPHA_PLAN S11b-2 / README-vs-Capabilities test (reused by S31)
    #[test]
    fn the_readme_support_table_agrees_with_the_driver_capabilities() {
        let capabilities = SimDriver::new().capabilities();

        let read = level_of("Read");
        assert_eq!(
            capabilities.read,
            read != "planned",
            "read: README says {read}"
        );
        assert_eq!(
            read, "simulator-tested",
            "read is claimed at the simulator level"
        );

        let write = level_of("Write");
        assert_eq!(
            capabilities.write,
            write != "planned",
            "write: README says {write}"
        );
        assert_eq!(
            write, "simulator-tested",
            "write is claimed at the simulator level"
        );

        let rest = level_of("Subscribe, browse and discover");
        assert_eq!(
            rest, "not supported",
            "the README says they are not supported"
        );
        assert_eq!(
            capabilities.subscribe,
            SubscriptionMode::Unsupported,
            "subscribe"
        );
        assert!(!capabilities.browse, "browse");
        assert!(!capabilities.discover, "discover");
    }

    #[test]
    fn every_support_cell_uses_one_of_the_allowed_labels() {
        let rows = support_rows();
        assert!(rows.len() >= 8, "the table was found: {} rows", rows.len());
        for (label, level) in rows {
            assert!(
                LEVELS.contains(&level.as_str()),
                "{label}: {level:?} is not an allowed support level"
            );
        }
    }
}
