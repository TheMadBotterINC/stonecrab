//! The crate README's support table agrees with `ModbusTcpDriver::capabilities()`.
//!
//! The table is the crate's public statement of what is supported and at which level
//! (SPEC.md 'Hardware evidence'). A capability the driver advertises must not say
//! `planned` or `not supported`, and one it does not advertise must not claim a level. The
//! test mirrors the simulator's `readme_matrix.rs`.
#![cfg(feature = "modbus")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab::modbus::ModbusTcpDriver;
    use stonecrab::{Driver, Operation, SubscriptionMode};

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
        include_str!("../docs/components/modbus.md")
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

    // covers: ALPHA_PLAN S26 / MB-TCP-CAPABILITIES README-vs-Capabilities test
    #[test]
    fn capabilities_readme_table_matches_value() {
        let capabilities = ModbusTcpDriver::new().capabilities();

        let read = level_of("Read");
        assert!(
            capabilities.read && !["planned", "not supported"].contains(&read.as_str()),
            "read: the driver advertises it and the README says {read}"
        );
        assert_eq!(
            read, "interoperability-tested",
            "read is claimed at the level the pymodbus job (S27) reached"
        );

        let ping = level_of("Ping");
        assert!(
            capabilities.ping && !["planned", "not supported"].contains(&ping.as_str()),
            "ping: the driver advertises it and the README says {ping}"
        );
        assert!(capabilities.supports(Operation::Ping), "ping operation");

        let write = level_of("Write");
        assert_eq!(
            write, "not supported",
            "the README says write is unsupported"
        );
        assert!(!capabilities.write, "write");
        assert!(!capabilities.supports(Operation::Write), "write operation");

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

        // The interoperability job (S27) exists; no hardware claim at all.
        assert_eq!(level_of("Interoperability"), "interoperability-tested");
        assert_eq!(level_of("Hardware verification"), "planned");
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
