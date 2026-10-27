//! Interoperability with pymodbus 3.8.6 (slice S27): the driver reads the register map
//! from a real Modbus TCP server in a container.
//!
//! Every test here is `#[ignore]`d and runs only when `STONECRAB_INTEROP_MODBUS` names the
//! server (for example `modbus-tcp://127.0.0.1:5020`): `mise run interop` starts the image
//! of `interop/pymodbus/` and runs them, and so does the `interop` job of
//! `.github/workflows/ci.yml`. Without the variable the tests stay ignored and nothing
//! here touches the network.
//!
//! The expected values are the `value` text of each entry of
//! `fixtures/modbus/interop/register-map.toml`, parsed here as the entry's type; they are
//! not the register words the server holds and not the output of any decoder under test.
//! A test therefore fails when the server serves the wrong words, when the driver decodes
//! them wrongly or when the wire address is off by one.
//!
//! Time: every read has a 2 s deadline taken from the clock the driver uses, no test
//! sleeps, and a failure to connect fails the test with a message that names the variable.
//!
//! # What the server's blocks hold
//! `interop/pymodbus/server.py` serves logical addresses `1..=N` for a register block of
//! size `N` in the map (it allocates `N + 1` slots, slot 0 being unreachable), and the
//! holding block is enlarged to cover the little-endian region at registers 300 and 301.
//! pymodbus 3.8.6 refuses an FC03 or FC04 read with exception 02 when `wire + 1 + count`
//! exceeds the slots of the block; the test that reads beyond the map relies on that rule
//! and says so. It does not validate FC01 and FC02 reads, so the coil and discrete-input
//! blocks cover the whole address space and no bit read can be refused.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use stonecrab_core::{
        AddressOptions, Clock, Connection, ConnectionConfig, Deadline, Driver, ErrorKind,
        ItemError, PlcValue, ProtocolStatus, ReadRequest, ReadResponse, ResourceLimits, Tag, TagId,
    };
    use stonecrab_modbus::{ModbusTcpDriver, SystemClock};
    use stonecrab_testkit::{
        CaseOutcome, ContractFixture, ContractReport, ContractSuite, CountingFactory,
        FixtureAddress, InteropMap, RegisterEntry,
    };
    use stonecrab_transport::TcpFactory;

    /// The environment variable that names the server.
    const VARIABLE: &str = "STONECRAB_INTEROP_MODBUS";

    /// The environment variable that names the evidence directory (optional).
    const EVIDENCE: &str = "STONECRAB_INTEROP_EVIDENCE_DIR";

    /// The deadline of every read.
    const DEADLINE: Duration = Duration::from_secs(2);

    /// The most items any request of these tests carries: 2001 coils.
    const MAX_ITEMS: usize = 2_001;

    fn server_url() -> String {
        std::env::var(VARIABLE).unwrap_or_else(|_| {
            panic!(
                "{VARIABLE} is not set: these tests need the pymodbus server of interop/pymodbus \
                 (run `mise run interop`, or start the container and set \
                 {VARIABLE}=modbus-tcp://127.0.0.1:5020)"
            )
        })
    }

    fn url_with(query: &str) -> String {
        let base = server_url();
        if query.is_empty() {
            base
        } else {
            format!("{base}?{query}")
        }
    }

    fn register_map() -> InteropMap {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/modbus/interop/register-map.toml");
        InteropMap::load(&path).expect("the interop register map loads")
    }

    /// The configuration with the request item budget raised, everything else kept.
    fn with_item_budget(config: &ConnectionConfig, items: usize) -> ConnectionConfig {
        let limits = ResourceLimits::builder()
            .max_items_per_request(items)
            .build()
            .expect("limits");
        let mut builder = ConnectionConfig::builder(config.scheme(), config.endpoint().clone())
            .timeouts(*config.timeouts())
            .reconnect(*config.reconnect())
            .retry(config.retry().clone())
            .access(config.access())
            .limits(limits);
        if let Some(name) = config.address_options().convention() {
            builder = builder.address_convention(name);
        }
        if let Some(options) = config.driver_options() {
            builder = builder.driver_options(Arc::clone(options));
        }
        builder.build().expect("the rebuilt configuration")
    }

    /// A driver, its connection to the server and the options addresses are parsed with.
    struct Peer {
        driver: ModbusTcpDriver,
        options: AddressOptions,
        connection: Box<dyn Connection>,
    }

    impl Peer {
        async fn connect(query: &str) -> Self {
            Self::open(ModbusTcpDriver::new(), query).await
        }

        async fn open(driver: ModbusTcpDriver, query: &str) -> Self {
            let uri = url_with(query);
            let config = driver
                .parse_config(&uri)
                .expect("the connection string parses");
            let config = with_item_budget(&config, MAX_ITEMS);
            let options = config.address_options();
            let connection = driver.connect(config).await.unwrap_or_else(|error| {
                panic!(
                    "cannot connect to the server named by {VARIABLE}: {error}. Start it with \
                     `mise run interop` or point the variable at a running pymodbus peer"
                )
            });
            Self {
                driver,
                options,
                connection,
            }
        }

        fn tag(&self, id: &str, text: &str) -> Tag {
            let address = self
                .driver
                .parse_address(text, &self.options)
                .expect("the address parses");
            Tag::new(TagId::new(id).expect("tag id"), address)
        }

        async fn read(&self, tags: Vec<Tag>) -> ReadResponse {
            let deadline = Deadline::after(SystemClock::default().now(), DEADLINE)
                .expect("a deadline that fits");
            let mut builder = ReadRequest::builder(self.connection.limits()).deadline(deadline);
            for tag in tags {
                builder = builder.tag(tag);
            }
            let request = builder.build().expect("the request is within its budgets");
            self.connection
                .read(request)
                .await
                .expect("the read completes")
        }

        async fn close(self) {
            self.connection.close().await.expect("close");
        }
    }

    /// The value an entry documents, parsed from its `value` text as the entry's type.
    fn expected(entry: &RegisterEntry) -> PlcValue {
        let text = entry.value.as_str();
        let bad = || format!("{} value `{text}` parses as {}", entry.row, entry.data_type);
        match entry.data_type.as_str() {
            "BOOL" => PlcValue::Bool(text == "true"),
            "BYTE" | "USINT" => PlcValue::U8(text.parse().unwrap_or_else(|_| panic!("{}", bad()))),
            "SINT" => PlcValue::I8(text.parse().unwrap_or_else(|_| panic!("{}", bad()))),
            "WORD" | "UINT" => PlcValue::U16(text.parse().unwrap_or_else(|_| panic!("{}", bad()))),
            "INT" => PlcValue::I16(text.parse().unwrap_or_else(|_| panic!("{}", bad()))),
            "DWORD" | "UDINT" => {
                PlcValue::U32(text.parse().unwrap_or_else(|_| panic!("{}", bad())))
            }
            "DINT" => PlcValue::I32(text.parse().unwrap_or_else(|_| panic!("{}", bad()))),
            "LWORD" | "ULINT" => {
                PlcValue::U64(text.parse().unwrap_or_else(|_| panic!("{}", bad())))
            }
            "LINT" => PlcValue::I64(text.parse().unwrap_or_else(|_| panic!("{}", bad()))),
            "REAL" => PlcValue::F32(text.parse().unwrap_or_else(|_| panic!("{}", bad()))),
            "LREAL" => PlcValue::F64(text.parse().unwrap_or_else(|_| panic!("{}", bad()))),
            other => panic!("{}: unknown type {other}", entry.row),
        }
    }

    fn outcome<'a>(response: &'a ReadResponse, id: &str) -> &'a Result<PlcValue, ItemError> {
        response
            .get(&TagId::new(id).expect("tag id"))
            .unwrap_or_else(|| panic!("the response has an item for {id}"))
            .outcome()
    }

    /// The value of a tag that was read, or `None` when its item failed.
    fn value_of(response: &ReadResponse, id: &str) -> Option<PlcValue> {
        outcome(response, id).as_ref().ok().cloned()
    }

    /// The error of a tag whose item failed; the test fails when it was read.
    fn failure<'a>(response: &'a ReadResponse, id: &str) -> &'a ItemError {
        outcome(response, id)
            .as_ref()
            .expect_err("the item was expected to fail")
    }

    fn entry_tag(peer: &Peer, area: &str, entry: &RegisterEntry) -> Tag {
        let text = format!("{area}:{}:{}", entry.address, entry.data_type);
        peer.tag(&format!("{area}-{}", entry.address), &text)
    }

    fn assert_entries(response: &ReadResponse, area: &str, entries: &[RegisterEntry]) {
        for entry in entries {
            let id = format!("{area}-{}", entry.address);
            assert_eq!(
                value_of(response, &id),
                Some(expected(entry)),
                "{} {} at {area} {}: the documented value `{}`",
                entry.row,
                entry.data_type,
                entry.address,
                entry.value
            );
        }
    }

    /// Read each bit of a run as its own tag and compare with the documented bits.
    async fn read_bits(area: &str, address: u16, documented: &[bool]) {
        let peer = Peer::connect("").await;
        let tags: Vec<Tag> = (0..documented.len())
            .map(|offset| {
                let logical = usize::from(address).saturating_add(offset);
                peer.tag(&format!("bit-{logical}"), &format!("{area}:{logical}"))
            })
            .collect();
        let response = peer.read(tags).await;
        let observed: Vec<Option<PlcValue>> = (0..documented.len())
            .map(|offset| {
                let logical = usize::from(address).saturating_add(offset);
                value_of(&response, &format!("bit-{logical}"))
            })
            .collect();
        let wanted: Vec<Option<PlcValue>> = documented
            .iter()
            .map(|bit| Some(PlcValue::Bool(*bit)))
            .collect();
        assert_eq!(observed, wanted, "{area} {address}..: the documented bits");
        assert_eq!(
            response.split_into().get(),
            1,
            "adjacent bits are one wire request"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-TCP-CONNECT,
    // MB-TCP-READ-HR, MB-INTEROP-HR-01 to MB-INTEROP-HR-07
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS"]
    async fn interop_reads_all_fifteen_holding_register_types() {
        let map = register_map();
        assert_eq!(map.holding_registers.len(), 15, "the map lists 15 types");
        let peer = Peer::connect("").await;

        // All fifteen in one request: registers 1 to 31 are contiguous, so one chunk.
        let tags = map
            .holding_registers
            .iter()
            .map(|entry| entry_tag(&peer, "holding-register", entry))
            .collect();
        let response = peer.read(tags).await;
        assert_entries(&response, "holding-register", &map.holding_registers);
        assert_eq!(
            response.split_into().get(),
            1,
            "registers 1..=31 are one request"
        );

        // And each type on its own, the way the upstream single-tag reads go.
        for entry in &map.holding_registers {
            let single = peer
                .read(vec![entry_tag(&peer, "holding-register", entry)])
                .await;
            assert_entries(&single, "holding-register", std::slice::from_ref(entry));
        }
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-TCP-READ-COIL, MB-INTEROP-COIL
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS"]
    async fn interop_reads_coils_1_to_10() {
        let map = register_map();
        read_bits("coil", map.coils.address, &map.coils.values).await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-TCP-READ-DI, MB-INTEROP-DI
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS"]
    async fn interop_reads_discrete_inputs_1_to_10() {
        let map = register_map();
        read_bits(
            "discrete-input",
            map.discrete_inputs.address,
            &map.discrete_inputs.values,
        )
        .await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-TCP-READ-IR, MB-INTEROP-IR
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS"]
    async fn interop_reads_input_registers_1_to_7() {
        let map = register_map();
        assert_eq!(
            map.input_registers.len(),
            5,
            "five typed entries over 1..=7"
        );
        let peer = Peer::connect("").await;
        let tags = map
            .input_registers
            .iter()
            .map(|entry| entry_tag(&peer, "input-register", entry))
            .collect();
        let response = peer.read(tags).await;
        assert_entries(&response, "input-register", &map.input_registers);
        assert_eq!(
            response.split_into().get(),
            1,
            "registers 1..=7 are one request"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-INTEROP-LE, MB-BO-LE (D8)
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS"]
    async fn interop_little_endian_region_decodes_real() {
        let map = register_map();
        let [entry] = map.holding_extension.as_slice() else {
            panic!("the map lists exactly one little-endian entry");
        };
        assert_eq!(entry.address, 300, "registers 300 and 301");
        assert_eq!(
            entry.byte_order.as_deref(),
            Some("LITTLE_ENDIAN"),
            "its order"
        );
        let want = expected(entry);
        let plain = format!("holding-register:{}:REAL", entry.address);

        // The address block overrides the order for this tag alone.
        let peer = Peer::connect("").await;
        let block = format!("{plain}{{byte-order: 'LITTLE_ENDIAN'}}");
        let by_block = peer.read(vec![peer.tag("by-block", &block)]).await;
        assert_eq!(
            value_of(&by_block, "by-block"),
            Some(want.clone()),
            "address block"
        );
        // The connection default is big-endian, so the same registers decode to something else.
        let by_default = peer.read(vec![peer.tag("by-default", &plain)]).await;
        assert!(
            matches!(
                value_of(&by_default, "by-default"),
                Some(PlcValue::F32(got)) if Some(PlcValue::F32(got)) != Some(want.clone())
            ),
            "the default order reads the region successfully, but not as 3.141593"
        );
        peer.close().await;

        // The connection parameter sets the order for every tag without its own.
        let little = Peer::connect("default-payload-byte-order=LITTLE_ENDIAN").await;
        let by_parameter = little.read(vec![little.tag("by-parameter", &plain)]).await;
        assert_eq!(
            value_of(&by_parameter, "by-parameter"),
            Some(want),
            "connection parameter"
        );
        little.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-TCP-READ-IR, MB-EXC-02,
    // MB-READ-MERGE (a chunk exception fails only its members)
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS"]
    async fn interop_read_beyond_map_yields_device_exception() {
        // Assumption about pymodbus 3.8.6, checked by this test: a block of N slots (the
        // map's size plus the unreachable slot 0 that server.py allocates) is read at wire
        // address W for C items only when W + 1 + C <= N. The input-register block has
        // 21 + 1 = 22 slots, so logical 22 (wire 21) is the first address past the map and
        // pymodbus answers exception 02 (ILLEGAL DATA ADDRESS).
        let map = register_map();
        let beyond = map.sizes.input_registers.saturating_add(1);
        assert_eq!(beyond, 22, "the first input register past the map");
        let peer = Peer::connect("").await;
        let response = peer
            .read(vec![
                peer.tag("good", "holding-register:13:UINT"),
                peer.tag("beyond", &format!("input-register:{beyond}:UINT")),
            ])
            .await;
        assert_eq!(
            value_of(&response, "good"),
            Some(PlcValue::U16(42424)),
            "the other chunk still succeeds"
        );
        let error = failure(&response, "beyond");
        assert_eq!(error.kind(), ErrorKind::InvalidAddress, "exception 02 kind");
        assert_eq!(
            error.status().map(ProtocolStatus::code),
            Some(2),
            "the device's exception code is ILLEGAL DATA ADDRESS"
        );
        assert_eq!(response.split_into().get(), 2, "two areas, two chunks");
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-READ-MERGE
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS"]
    async fn interop_split_batch_discloses_chunks() {
        let map = register_map();
        let peer = Peer::connect("").await;

        // Two areas are two chunks, and the answers come back in request order.
        let two_areas = peer
            .read(vec![
                peer.tag("hr", "holding-register:13:UINT"),
                peer.tag("ir", "input-register:3:UINT"),
                peer.tag("hr2", "holding-register:12:INT"),
            ])
            .await;
        assert_eq!(
            two_areas.split_into().get(),
            2,
            "holding and input are two requests"
        );
        let order: Vec<String> = two_areas
            .items()
            .iter()
            .map(|item| item.tag().to_string())
            .collect();
        assert_eq!(order, ["hr", "ir", "hr2"], "request order is kept");
        assert_eq!(value_of(&two_areas, "hr"), Some(PlcValue::U16(42424)), "hr");
        assert_eq!(value_of(&two_areas, "ir"), Some(PlcValue::U16(42424)), "ir");
        assert_eq!(
            value_of(&two_areas, "hr2"),
            Some(PlcValue::I16(-2424)),
            "hr2"
        );

        // More than 125 registers of one area are two chunks: 125 and 1.
        let tags: Vec<Tag> = (1..=126_u16)
            .map(|address| {
                peer.tag(
                    &format!("r{address}"),
                    &format!("holding-register:{address}:UINT"),
                )
            })
            .collect();
        let many = peer.read(tags).await;
        assert_eq!(many.split_into().get(), 2, "126 registers split in two");
        let mut words: Vec<u16> = vec![0; 127];
        for entry in &map.holding_registers {
            for (offset, word) in entry.words.iter().enumerate() {
                let slot = usize::from(entry.address).saturating_add(offset);
                if let Some(target) = words.get_mut(slot) {
                    *target = *word;
                }
            }
        }
        for address in 1..=126_usize {
            let want = words.get(address).copied().unwrap_or_default();
            assert_eq!(
                value_of(&many, &format!("r{address}")),
                Some(PlcValue::U16(want)),
                "register {address} across the chunk boundary"
            );
        }
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-LIMIT-REGISTERS,
    // MB-LIMIT-COILS, MB-READ-MERGE
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS"]
    async fn interop_125_register_and_2000_coil_reads() {
        let map = register_map();
        let peer = Peer::connect("").await;

        let registers: Vec<Tag> = (1..=125_u16)
            .map(|address| {
                peer.tag(
                    &format!("r{address}"),
                    &format!("holding-register:{address}:UINT"),
                )
            })
            .collect();
        let registers = peer.read(registers).await;
        assert_eq!(
            registers.split_into().get(),
            1,
            "125 registers are one request"
        );
        assert_eq!(registers.iter_ok().count(), 125, "all 125 answered");

        let coils = |count: u16| -> Vec<Tag> {
            (1..=count)
                .map(|address| peer.tag(&format!("c{address}"), &format!("coil:{address}")))
                .collect()
        };
        let all = peer.read(coils(2000)).await;
        assert_eq!(all.split_into().get(), 1, "2000 coils are one request");
        assert_eq!(all.iter_ok().count(), 2000, "all 2000 answered");
        for (offset, bit) in map.coils.values.iter().enumerate() {
            let logical = usize::from(map.coils.address).saturating_add(offset);
            assert_eq!(
                value_of(&all, &format!("c{logical}")),
                Some(PlcValue::Bool(*bit)),
                "coil {logical} inside the 2000-coil answer"
            );
        }
        assert_eq!(
            value_of(&all, "c2000"),
            Some(PlcValue::Bool(false)),
            "the last coil of the map"
        );

        // One past each maximum splits into a full chunk and a chunk of one. The server's
        // coil block covers the whole address space (see the module documentation), so the
        // 2001st coil is answered and is false.
        let past_registers = peer
            .read(
                (1..=126_u16)
                    .map(|address| {
                        peer.tag(
                            &format!("p{address}"),
                            &format!("holding-register:{address}:UINT"),
                        )
                    })
                    .collect(),
            )
            .await;
        assert_eq!(past_registers.split_into().get(), 2, "126 registers split");
        assert_eq!(
            past_registers.iter_ok().count(),
            126,
            "both chunks answered"
        );
        let past_coils = peer.read(coils(2001)).await;
        assert_eq!(past_coils.split_into().get(), 2, "2001 coils split in two");
        assert_eq!(past_coils.iter_ok().count(), 2001, "both chunks answered");
        assert_eq!(
            value_of(&past_coils, "c2001"),
            Some(PlcValue::Bool(false)),
            "the coil of the second chunk"
        );
        peer.close().await;
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-ADDR-BARE-40001
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS"]
    async fn interop_bare_40001_reads_with_explicit_convention() {
        // Without the convention the bare spelling is ambiguous and is refused at parse time.
        let plain = ModbusTcpDriver::new();
        let Err(refused) = plain.parse_address("40001:BOOL", &AddressOptions::default()) else {
            panic!("a bare address needs the explicit convention");
        };
        assert_eq!(
            refused.kind(),
            ErrorKind::InvalidAddress,
            "refused without it"
        );

        // With address-convention=plc4x the same text is holding register 1 (true), and the
        // other bare areas read the map's values.
        let peer = Peer::connect("address-convention=plc4x").await;
        let response = peer
            .read(vec![
                peer.tag("hr1", "40001:BOOL"),
                peer.tag("hr13", "400013:UINT"),
                peer.tag("ir3", "30003:UINT"),
                peer.tag("coil1", "00001"),
                peer.tag("di2", "10002"),
            ])
            .await;
        assert_eq!(
            value_of(&response, "hr1"),
            Some(PlcValue::Bool(true)),
            "40001"
        );
        assert_eq!(
            value_of(&response, "hr13"),
            Some(PlcValue::U16(42424)),
            "400013"
        );
        assert_eq!(
            value_of(&response, "ir3"),
            Some(PlcValue::U16(42424)),
            "30003"
        );
        assert_eq!(
            value_of(&response, "coil1"),
            Some(PlcValue::Bool(true)),
            "00001"
        );
        assert_eq!(
            value_of(&response, "di2"),
            Some(PlcValue::Bool(true)),
            "10002"
        );
        peer.close().await;
    }

    // The suite sentences that cannot run against a server that has no fault hooks, with the
    // reason the report must give for each.
    const SKIPPED: [(&str, &str); 9] = [
        (
            "an absent deadline uses the connection timeout",
            "fixture cannot inject timeout",
        ),
        (
            "a disconnect during a read yields Disconnected",
            "fixture cannot inject disconnect",
        ),
        (
            "a timeout carries diagnostics naming operation, state and category",
            "fixture cannot inject timeout",
        ),
        (
            "a disconnect carries diagnostics naming operation, state and category",
            "fixture cannot inject disconnect",
        ),
        (
            "a device exception carries diagnostics naming operation, state and category",
            "fixture cannot inject device exception",
        ),
        (
            "read retry stops after the configured maximum attempts",
            "fixture cannot inject timeout",
        ),
        (
            "read retry skips non-retryable kinds",
            "fixture cannot inject device exception",
        ),
        (
            "a read-only policy rejects a write with no I/O",
            "driver reports write unsupported",
        ),
        (
            "an unknown write outcome is never replayed",
            "driver reports write unsupported",
        ),
    ];

    /// Where the contract report goes: the evidence directory when the variable names one.
    fn evidence_dir() -> PathBuf {
        std::env::var_os(EVIDENCE).map_or_else(
            || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/interop-evidence"),
            PathBuf::from,
        )
    }

    fn write_report(url: &str, report: &ContractReport, io: (u64, u64, u64)) -> PathBuf {
        let directory = evidence_dir();
        std::fs::create_dir_all(&directory).expect("the evidence directory");
        let path = directory.join("contract-report.txt");
        let mut file = std::fs::File::create(&path).expect("the report file");
        writeln!(file, "server: {url}").expect("write");
        writeln!(file, "driver: ModbusTcpDriver::new() over TcpFactory").expect("write");
        writeln!(
            file,
            "transport: {} opens, {} frames, {} bytes sent\n\n{report}",
            io.0, io.1, io.2
        )
        .expect("write");
        writeln!(
            file,
            "skipped by name (no fault hooks against a real server):"
        )
        .expect("write");
        for (name, reason) in SKIPPED {
            writeln!(file, "  - {name}: {reason}").expect("write");
        }
        path
    }

    // covers: SPEC mandatory-failure-cases / F5 Modbus level; MB-CONTRACT-SUITE, MB-TCP-CONNECT
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "interop: set STONECRAB_INTEROP_MODBUS"]
    async fn interop_contract_suite_passes_against_pymodbus() {
        // Reach the server first so a missing one fails with the message naming the variable.
        Peer::connect("").await.close().await;

        let uri = server_url();
        let counting = CountingFactory::new(Arc::new(TcpFactory::default()));
        let driver = ModbusTcpDriver::new().with_transport_factory(Arc::new(counting.clone()));
        // The probe moves with every open and every frame the driver sends, so "no I/O"
        // cases are exact over a real socket; the reads counter is the frames sent.
        let probe = counting.clone();
        let frames = counting.clone();
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::default());
        let fixture = ContractFixture::new(
            clock,
            "holding-register:13:UINT",
            // Parses and lies far outside the holding block, so the server answers 02.
            "holding-register:60000:UINT",
            Arc::new(FixtureAddress::new("elsewhere", None)),
            move || probe.io(),
            move || u32::try_from(frames.frames()).unwrap_or(u32::MAX),
        )
        // Naming a writable address lets the unsupported-write case run; writes stay refused.
        .with_writable("holding-register:13:UINT", PlcValue::U16(1));
        // No fault hooks: the server is a stock pymodbus and cannot drop a link, swallow a
        // reply or refuse on demand, and timing over a real clock stays unverified (D21).

        let report = ContractSuite::run(&driver, &uri, &fixture)
            .await
            .expect("the suite always resolves");
        let path = write_report(
            &uri,
            &report,
            (counting.opens(), counting.frames(), counting.bytes()),
        );
        writeln!(
            std::io::stdout().lock(),
            "{uri}\n{report}\nreport: {}",
            path.display()
        )
        .expect("stdout");

        let skipped: Vec<&str> = SKIPPED.iter().map(|(name, _)| *name).collect();
        assert!(
            report.is_green(),
            "the suite is green against pymodbus; skipped by name: {skipped:?}\n{report}"
        );
        assert_eq!(report.failed(), 0, "no failures:\n{report}");
        assert_eq!(
            report.passed().saturating_add(report.skipped()),
            ContractSuite::cases().len(),
            "every case has an outcome:\n{report}"
        );
        assert_eq!(
            report.skipped(),
            SKIPPED.len(),
            "only the named skips:\n{report}"
        );
        for (name, reason) in SKIPPED {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Skipped(reason.to_owned())),
                "{name} is skipped because: {reason}\n{report}"
            );
        }
        assert!(
            counting.opens() > 0 && counting.frames() > 0,
            "the suite did real I/O"
        );
        assert!(
            path.is_file(),
            "the report was written to {}",
            path.display()
        );
    }
}
