//! The repository's synthetic profiles under `profiles/` (slice S30): each one loads,
//! validates, declares the address convention its scheme needs, has a twin for the other
//! driver and is listed in `profiles/README.md`. The profiles are repository data, read
//! from the workspace root relative to this crate; they are not packaged.
#![cfg(all(feature = "sim", feature = "sensors"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use stonecrab::PlcValueType;
    use stonecrab::sensors::{SensorProfile, ValidationStatus};
    use stonecrab::sim::SimDriver;

    /// The `profiles/` directory of the workspace root.
    fn profiles_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles")
    }

    /// Every `.toml` below `dir`, as a path relative to `profiles/` with `/` separators,
    /// paired with its text, in sorted order.
    fn profile_files(dir: &Path, relative: &str) -> Vec<(String, String)> {
        let mut found = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .expect("the profiles directory is readable")
            .map(|entry| entry.expect("a directory entry").path())
            .collect();
        entries.sort();
        for path in entries {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a UTF-8 file name")
                .to_owned();
            let relative_name = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            if path.is_dir() {
                found.extend(profile_files(&path, &relative_name));
            } else if path
                .extension()
                .is_some_and(|extension| extension == "toml")
            {
                let text = std::fs::read_to_string(&path).expect("a readable profile");
                found.push((relative_name, text));
            }
        }
        found
    }

    fn all_profiles() -> Vec<(String, SensorProfile)> {
        let files = profile_files(&profiles_dir(), "");
        assert!(!files.is_empty(), "profiles/ holds profile files");
        files
            .into_iter()
            .map(|(name, text)| {
                let profile = SensorProfile::from_toml(&text)
                    .unwrap_or_else(|error| panic!("{name} does not load: {error}"));
                (name, profile)
            })
            .collect()
    }

    // covers: SENSORS-SYNTHETIC-PROFILES / every repository profile loads and validates
    #[test]
    fn every_repository_profile_loads_and_validates_as_synthetic() {
        let profiles = all_profiles();
        assert_eq!(profiles.len(), 6, "three devices in two variants");
        for (name, profile) in &profiles {
            assert!(
                name.starts_with("synthetic/"),
                "{name} is under profiles/synthetic/"
            );
            assert_eq!(profile.validation, ValidationStatus::Synthetic, "{name}");
            assert_eq!(profile.validate(), Ok(()), "{name} validates");
            assert!(!profile.measurements.is_empty(), "{name} has measurements");
            assert!(
                !profile.provenance.source.is_empty(),
                "{name} names its provenance"
            );
        }
        let ids: BTreeSet<_> = profiles.iter().map(|(_, p)| p.id.as_str()).collect();
        assert_eq!(ids.len(), profiles.len(), "profile ids are unique");
    }

    // covers: SENSORS-SYNTHETIC-PROFILES / the variants declare their scheme and convention
    #[test]
    fn modbus_profiles_declare_the_plc4x_convention_and_sim_profiles_none() {
        for (name, profile) in all_profiles() {
            if let Some(file) = name.strip_prefix("synthetic/modbus-tcp/") {
                assert_eq!(profile.connection.scheme, "modbus-tcp", "{file} scheme");
                assert_eq!(
                    profile.connection.address_convention.as_deref(),
                    Some("plc4x"),
                    "{file} declares the plc4x convention"
                );
            } else if let Some(file) = name.strip_prefix("synthetic/sim/") {
                assert_eq!(profile.connection.scheme, "sim", "{file} scheme");
                assert_eq!(
                    profile.connection.address_convention, None,
                    "{file} declares no convention"
                );
                assert!(
                    profile.id.as_str().ends_with("_sim"),
                    "{file}: a sim twin's id ends in _sim"
                );
            } else {
                panic!("{name} is under neither synthetic/modbus-tcp/ nor synthetic/sim/");
            }
        }
    }

    // covers: SENSORS-SYNTHETIC-PROFILES / each device has a twin with the same measurements
    #[test]
    fn every_modbus_profile_has_a_sim_twin_with_the_same_measurement_names_and_units() {
        let profiles = all_profiles();
        let modbus: Vec<_> = profiles
            .iter()
            .filter_map(|(name, profile)| {
                name.strip_prefix("synthetic/modbus-tcp/")
                    .map(|file| (file, profile))
            })
            .collect();
        assert_eq!(modbus.len(), 3, "three modbus-tcp profiles");
        for (file, profile) in modbus {
            let twin_name = format!("synthetic/sim/{file}");
            let (_, twin) = profiles
                .iter()
                .find(|(name, _)| *name == twin_name)
                .unwrap_or_else(|| panic!("{file} has no sim twin"));
            assert_eq!(
                format!("{}_sim", profile.id.as_str()),
                twin.id.as_str(),
                "{file}: the twin's id is the id with a _sim suffix"
            );
            let describe = |p: &SensorProfile| {
                p.measurements
                    .iter()
                    .map(|m| {
                        let status: Vec<_> = m
                            .status_fields
                            .iter()
                            .map(|f| (f.name.clone(), f.invert))
                            .collect();
                        (
                            m.name.as_str().to_owned(),
                            m.unit.clone(),
                            format!("{:?}", m.range),
                            status,
                            format!("{:?}", m.quality_map),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                describe(profile),
                describe(twin),
                "{file}: same names, units, ranges, status fields and quality mapping"
            );
        }
    }

    // covers: SENSORS-SYNTHETIC-PROFILES / the sim variants build tags for the simulator
    #[test]
    fn the_sim_profiles_build_tags_for_the_simulator_driver() {
        let driver = SimDriver::new();
        for (name, profile) in all_profiles() {
            if !name.starts_with("synthetic/sim/") {
                continue;
            }
            let tags = profile
                .tags(&driver)
                .unwrap_or_else(|error| panic!("{name} builds no tags: {error}"));
            let status_tags: usize = profile
                .measurements
                .iter()
                .map(|m| m.status_fields.len())
                .sum();
            assert_eq!(
                tags.len(),
                profile.measurements.len() + status_tags,
                "{name}: one tag per measurement and per status field"
            );
            assert!(
                tags.iter().all(|tag| tag.value_type().is_some()),
                "{name}: the simulator types every address"
            );
        }
        // A temperature reading is a float on the wire the profile is a twin of.
        let temperature = all_profiles()
            .into_iter()
            .find(|(name, _)| name == "synthetic/sim/temperature_transmitter.toml")
            .map(|(_, profile)| profile)
            .expect("the sim temperature profile exists");
        let tags = temperature.tags(&driver).expect("tags");
        assert_eq!(
            tags.first().and_then(stonecrab::Tag::value_type),
            Some(PlcValueType::F32)
        );
    }

    // covers: SENSORS-SYNTHETIC-PROFILES / profiles/README.md lists every profile file
    #[test]
    fn the_profiles_readme_lists_every_profile_file() {
        let readme = std::fs::read_to_string(profiles_dir().join("README.md"))
            .expect("profiles/README.md exists");
        for (name, _) in all_profiles() {
            let relative = name.strip_prefix("synthetic/").unwrap_or(&name);
            let heading = format!("### synthetic/{relative}");
            assert!(
                readme.lines().any(|line| line.trim_end() == heading),
                "profiles/README.md has no section headed {heading}"
            );
        }
        assert!(
            readme.contains("no named device profile"),
            "the README states that no named device profile exists"
        );
    }
}
