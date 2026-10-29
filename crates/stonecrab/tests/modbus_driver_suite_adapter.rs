//! The driver-suite XML adapter (slice S04b, SPEC.md "Code generation and upstream
//! maintenance", requirement 7).
//!
//! Three retained PLC4X files carry the exchanges of the Modbus TCP driver tests as
//! XML-serialized `ModbusADU` models: `DriverTestsuite.xml`, `Modbus-all-datatypes.xml` and
//! `Modbus-all-datatypes-little-endian.xml` (`fixtures/modbus/upstream/tcp/`). The adapter
//! below reads the models with `roxmltree`, serializes each read exchange to wire bytes with
//! its own serializer that follows `protocols/modbus/.../modbus.mspec` (it never calls
//! Stonecrab's codec or decoder, so a codec bug cannot hide in its own vectors) and renders
//! the `.hex`, the `.expected.json` and the `PROVENANCE.toml` entry of every request and
//! response from the same model. `roxmltree` is a dev-dependency only: the adapter is a test
//! tool, it never ships in the crate (ADR-0009 addendum, S04b).
//!
//! The mspec derives two fields that the XML also spells out, the ADU `length` (the PDU
//! length plus one, modbus.mspec:48) and every `byteCount` (`COUNT(value)`); the adapter
//! derives them and fails when the XML disagrees. It fails the same way on any shape it does not know (a raw
//! bytes step, an exception PDU, an unlisted PDU, a width that differs from the mspec).
//!
//! `adapter_serializes_each_driver_suite_adu_deterministically` re-runs the adapter and
//! compares the output with the committed files byte for byte. To regenerate after a deliberate
//! change, run that test once with the variable `STONECRAB_REGENERATE_ADAPTER_VECTORS` set;
//! it rewrites the vectors and the adapter-generated block of `PROVENANCE.toml`, and a stale
//! file it no longer emits has to be deleted by hand (the test names it).
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use roxmltree::{Document, Node};

    type Result<T> = std::result::Result<T, String>;

    const COMMIT: &str = "ea229918f5d07700e04d7d69ab1645da679cb700";
    const SHORT_COMMIT: &str = "ea229918";
    const UPSTREAM_DIR: &str = "protocols/modbus/src/test/resources/protocols/modbus/tcp";
    const REGENERATE: &str = "STONECRAB_REGENERATE_ADAPTER_VECTORS";
    const BEGIN: &str = "# BEGIN adapter-generated entries (slice S04b)";
    const END: &str = "# END adapter-generated entries (slice S04b)";
    /// The payload byte order upstream uses when a suite configures none.
    const DEFAULT_MODE: &str = "BIG_ENDIAN";
    const MODES: [&str; 4] = [
        DEFAULT_MODE,
        "LITTLE_ENDIAN",
        "BIG_ENDIAN_BYTE_SWAP",
        "LITTLE_ENDIAN_BYTE_SWAP",
    ];

    /// A retained upstream file and the slug its vectors carry.
    struct Suite {
        slug: &'static str,
        file: &'static str,
    }

    const SUITES: [Suite; 3] = [
        Suite {
            slug: "driver-suite",
            file: "DriverTestsuite.xml",
        },
        Suite {
            slug: "all-datatypes",
            file: "Modbus-all-datatypes.xml",
        },
        Suite {
            slug: "all-datatypes-little-endian",
            file: "Modbus-all-datatypes-little-endian.xml",
        },
    ];

    // ---- the mspec: the ADU and the PDUs the retained files use ---------------------

    /// How one field of a PDU is written on the wire.
    #[derive(Clone, Copy)]
    enum Kind {
        /// `simple uint 16`.
        Word,
        /// `implicit uint 8 byteCount 'COUNT(value)'`.
        ByteCount,
        /// `array byte value count 'byteCount'`.
        Bytes,
    }

    /// One `typeSwitch` case of `ModbusPDU` (modbus.mspec:78-276), in field order.
    struct Shape {
        element: &'static str,
        function: u8,
        request: bool,
        fields: &'static [(&'static str, Kind)],
    }

    /// Exactly the PDUs that occur in the three files; any other element is an unknown shape.
    const SHAPES: [Shape; 5] = [
        // modbus.mspec:135-138
        Shape {
            element: "ModbusPDUReadHoldingRegistersRequest",
            function: 0x03,
            request: true,
            fields: &[("startingAddress", Kind::Word), ("quantity", Kind::Word)],
        },
        // modbus.mspec:139-142
        Shape {
            element: "ModbusPDUReadHoldingRegistersResponse",
            function: 0x03,
            request: false,
            fields: &[("byteCount", Kind::ByteCount), ("value", Kind::Bytes)],
        },
        // modbus.mspec:144-147
        Shape {
            element: "ModbusPDUWriteSingleRegisterRequest",
            function: 0x06,
            request: true,
            fields: &[("address", Kind::Word), ("value", Kind::Word)],
        },
        // modbus.mspec:153-158
        Shape {
            element: "ModbusPDUWriteMultipleHoldingRegistersRequest",
            function: 0x10,
            request: true,
            fields: &[
                ("startingAddress", Kind::Word),
                ("quantity", Kind::Word),
                ("byteCount", Kind::ByteCount),
                ("value", Kind::Bytes),
            ],
        },
        // modbus.mspec:159-162
        Shape {
            element: "ModbusPDUWriteMultipleHoldingRegistersResponse",
            function: 0x10,
            request: false,
            fields: &[("startingAddress", Kind::Word), ("quantity", Kind::Word)],
        },
    ];

    /// A parsed field of a PDU.
    enum Field {
        Word(&'static str, u16),
        Count(u8),
        Bytes(Vec<u8>),
    }

    struct Pdu {
        function: u8,
        request: bool,
        fields: Vec<Field>,
        /// The function byte, then the fields (the mspec's `ModbusPDU`, error flag clear).
        bytes: Vec<u8>,
    }

    impl Pdu {
        fn word(&self, name: &str) -> Result<u16> {
            self.fields
                .iter()
                .find_map(|field| match field {
                    Field::Word(candidate, value) if *candidate == name => Some(*value),
                    Field::Word(..) | Field::Count(_) | Field::Bytes(_) => None,
                })
                .ok_or_else(|| format!("the PDU has no word `{name}`"))
        }

        fn value(&self) -> Result<&[u8]> {
            self.fields
                .iter()
                .find_map(|field| match field {
                    Field::Bytes(bytes) => Some(bytes.as_slice()),
                    Field::Word(..) | Field::Count(_) => None,
                })
                .ok_or_else(|| "the PDU has no byte array".to_owned())
        }
    }

    /// A whole Modbus TCP ADU serialized from its XML model.
    struct Adu {
        transaction_id: u16,
        unit_id: u8,
        pdu: Pdu,
        /// Header and PDU.
        bytes: Vec<u8>,
    }

    /// An outgoing or incoming step: the step's `name` attribute and its ADU.
    struct Step {
        name: String,
        adu: Adu,
    }

    // ---- reading the XML --------------------------------------------------------------

    fn name_of<'i>(node: Node<'_, 'i>) -> &'i str {
        node.tag_name().name()
    }

    fn elements<'a, 'i>(node: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
        node.children().filter(Node::is_element).collect()
    }

    fn text_of<'a>(node: Node<'a, '_>) -> &'a str {
        node.text().unwrap_or_default().trim()
    }

    /// The one child element called `name`.
    fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Result<Node<'a, 'i>> {
        let mut found = elements(node)
            .into_iter()
            .filter(|candidate| name_of(*candidate) == name);
        match (found.next(), found.next()) {
            (Some(only), None) => Ok(only),
            _ => Err(format!(
                "<{}> has not exactly one <{name}> child",
                name_of(node)
            )),
        }
    }

    /// The child elements, which must be exactly `names` in that order.
    fn sequence<'a, 'i, const N: usize>(
        node: Node<'a, 'i>,
        names: [&str; N],
    ) -> Result<[Node<'a, 'i>; N]> {
        let kids = elements(node);
        let found: Vec<&str> = kids.iter().map(|kid| name_of(*kid)).collect();
        if found.as_slice() != names.as_slice() {
            return Err(format!(
                "<{}> holds [{}], the shape known is [{}]",
                name_of(node),
                found.join(", "),
                names.join(", ")
            ));
        }
        <[Node<'a, 'i>; N]>::try_from(kids)
            .map_err(|kids| format!("<{}>: {} children", name_of(node), kids.len()))
    }

    /// The text of a field element whose `dataType` and `bitLength` are as the mspec says.
    fn typed<'a>(node: Node<'a, '_>, data_type: &str, bits: u32) -> Result<&'a str> {
        let width = bits.to_string();
        if node.attribute("dataType") != Some(data_type)
            || node.attribute("bitLength") != Some(width.as_str())
        {
            return Err(format!(
                "<{}> is not {data_type} with bitLength {bits} as modbus.mspec has it",
                name_of(node)
            ));
        }
        Ok(text_of(node))
    }

    fn number(node: Node<'_, '_>, data_type: &str, bits: u32) -> Result<u64> {
        let text = typed(node, data_type, bits)?;
        text.parse().map_err(|error| {
            format!(
                "<{}> holds `{text}`, not a decimal number: {error}",
                name_of(node)
            )
        })
    }

    fn word(node: Node<'_, '_>) -> Result<u16> {
        u16::try_from(number(node, "uint", 16)?)
            .map_err(|error| format!("<{}>: {error}", name_of(node)))
    }

    /// Hexadecimal digits, two per byte, without a prefix.
    fn hex_bytes(digits: &str) -> Result<Vec<u8>> {
        digits
            .as_bytes()
            .chunks(2)
            .map(|pair| {
                std::str::from_utf8(pair)
                    .ok()
                    .filter(|text| text.len() == 2)
                    .and_then(|text| u8::from_str_radix(text, 16).ok())
                    .ok_or_else(|| format!("`{digits}` is not whole hexadecimal bytes"))
            })
            .collect()
    }

    /// A `dataType="byte"` array such as `0x40490fdb`.
    fn byte_array(node: Node<'_, '_>) -> Result<Vec<u8>> {
        let digits = text_of(node)
            .strip_prefix("0x")
            .filter(|_| node.attribute("dataType") == Some("byte"))
            .ok_or_else(|| format!("<{}> is not a 0x byte array", name_of(node)))?;
        let bytes = hex_bytes(digits)?;
        let declared = node.attribute("bitLength").and_then(|v| v.parse().ok());
        if declared != bytes.len().checked_mul(8) {
            return Err(format!(
                "<{}>: bitLength does not match the {} bytes written",
                name_of(node),
                bytes.len()
            ));
        }
        Ok(bytes)
    }

    // ---- serializing the model ------------------------------------------------------------

    fn parse_pdu(pdu: Node<'_, '_>, request: bool) -> Result<Pdu> {
        let kids = elements(pdu);
        let [error, function, variant] = kids.as_slice() else {
            return Err("<ModbusPDU> does not hold errorFlag, functionFlag and one variant".into());
        };
        if name_of(*error) != "errorFlag" || name_of(*function) != "functionFlag" {
            return Err("<ModbusPDU> does not start with errorFlag and functionFlag".into());
        }
        if typed(*error, "bit", 1)? != "false" {
            return Err(
                "an exception PDU (ModbusPDUError) is not a shape this adapter knows".into(),
            );
        }
        let code = u8::try_from(number(*function, "uint", 7)?)
            .map_err(|error| format!("functionFlag: {error}"))?;
        let shape = SHAPES
            .iter()
            .find(|shape| shape.element == name_of(*variant))
            .ok_or_else(|| format!("unknown PDU <{}>", name_of(*variant)))?;
        if shape.function != code || shape.request != request {
            return Err(format!(
                "<{}> does not match function {code} or the step direction",
                shape.element
            ));
        }
        let members = elements(*variant);
        if members.len() != shape.fields.len() {
            return Err(format!(
                "<{}> has the wrong number of fields",
                shape.element
            ));
        }
        let mut fields = Vec::new();
        for ((name, kind), node) in shape.fields.iter().zip(&members) {
            if name_of(*node) != *name {
                return Err(format!("<{}>: expected field {name}", shape.element));
            }
            fields.push(match kind {
                Kind::Word => Field::Word(name, word(*node)?),
                Kind::ByteCount => Field::Count(
                    u8::try_from(number(*node, "uint", 8)?)
                        .map_err(|error| format!("{name}: {error}"))?,
                ),
                Kind::Bytes => Field::Bytes(byte_array(*node)?),
            });
        }
        let bytes = pdu_bytes(code, &fields)?;
        Ok(Pdu {
            function: code,
            request,
            fields,
            bytes,
        })
    }

    /// The wire form of a PDU; each `byteCount` must equal the length of the array it counts
    /// (`COUNT(value)`, modbus.mspec:141).
    fn pdu_bytes(function: u8, fields: &[Field]) -> Result<Vec<u8>> {
        let counted = fields.iter().find_map(|field| match field {
            Field::Bytes(array) => Some(array.len()),
            Field::Word(..) | Field::Count(_) => None,
        });
        let mut bytes = vec![function];
        for field in fields {
            match field {
                Field::Word(_, value) => bytes.extend(value.to_be_bytes()),
                Field::Count(count) if Some(usize::from(*count)) == counted => bytes.push(*count),
                Field::Count(count) => {
                    return Err(format!(
                        "byteCount is {count} but modbus.mspec derives COUNT(value) from the array"
                    ));
                }
                Field::Bytes(array) => bytes.extend(array),
            }
        }
        Ok(bytes)
    }

    /// Serialize a `ModbusADU` model (modbus.mspec:36-58, `MODBUS_TCP`).
    fn parse_adu(model: Node<'_, '_>, request: bool) -> Result<Adu> {
        let tcp = child(model, "ModbusTcpADU")?;
        let [transaction, protocol, length, unit, pdu] = sequence(
            tcp,
            [
                "transactionIdentifier",
                "protocolIdentifier",
                "length",
                "unitIdentifier",
                "pdu",
            ],
        )?;
        let transaction_id = word(transaction)?;
        if number(protocol, "uint", 16)? != 0 {
            return Err("protocolIdentifier is const 0x0000 (modbus.mspec:44)".into());
        }
        let unit_id = u8::try_from(number(unit, "uint", 8)?)
            .map_err(|error| format!("unitIdentifier: {error}"))?;
        let pdu = parse_pdu(child(pdu, "ModbusPDU")?, request)?;
        let derived = u16::try_from(pdu.bytes.len())
            .ok()
            .and_then(|len| len.checked_add(1))
            .ok_or("the PDU does not fit the length field")?;
        let declared = word(length)?;
        if declared != derived {
            return Err(format!(
                "length is {declared} but modbus.mspec:48 derives {derived} (pdu.lengthInBytes + 1)"
            ));
        }
        let mut bytes = Vec::new();
        bytes.extend(transaction_id.to_be_bytes());
        bytes.extend([0, 0]);
        bytes.extend(derived.to_be_bytes());
        bytes.push(unit_id);
        bytes.extend(&pdu.bytes);
        Ok(Adu {
            transaction_id,
            unit_id,
            pdu,
            bytes,
        })
    }

    /// An `outgoing-plc-message` (request) or `incoming-plc-message` (response) step. Upstream
    /// leaves the `name` attribute empty in the pcap-derived cases, so such a step is labelled
    /// with its element and its 1-based position among the steps of the case.
    fn parse_step(step: Node<'_, '_>, request: bool) -> Result<Step> {
        let name = match step
            .attribute("name")
            .ok_or("a PLC message step has no name")?
        {
            "" => format!(
                "unnamed {} at position {}",
                name_of(step),
                step.prev_siblings()
                    .filter(Node::is_element)
                    .count()
                    .saturating_add(1)
            ),
            named => named.to_owned(),
        };
        let [arguments, model] = sequence(step, ["parser-arguments", "ModbusADU"])?;
        let [driver, response] = sequence(arguments, ["driverType", "response"])?;
        if text_of(driver) != "MODBUS_TCP" || text_of(response) != (!request).to_string() {
            return Err(format!(
                "step `{name}`: parser-arguments are not the TCP direction"
            ));
        }
        let adu = parse_adu(model, request).map_err(|error| format!("step `{name}`: {error}"))?;
        Ok(Step { name, adu })
    }

    // ---- the cases ------------------------------------------------------------------------

    struct Tag {
        name: String,
        address: String,
        data_type: String,
        elements: u16,
        start: u16,
    }

    struct Value {
        data_type: String,
        text: String,
    }

    /// One read exchange: a request, its response, the tag it serves and the values the case
    /// expects the API to report.
    struct Read {
        case: String,
        exchange: usize,
        tag: Tag,
        values: Vec<Value>,
        request: Step,
        response: Step,
    }

    /// The steps of a case: the `api-request` and `api-response` elements around the
    /// serialized message pairs.
    struct Steps<'a, 'i> {
        api_request: Node<'a, 'i>,
        exchanges: Vec<(Step, Step)>,
        api_response: Node<'a, 'i>,
    }

    fn parse_steps<'a, 'i>(case: Node<'a, 'i>) -> Result<Steps<'a, 'i>> {
        let steps = elements(child(case, "steps")?);
        let Some((last, rest)) = steps.split_last() else {
            return Err("a case has no steps".into());
        };
        let Some((first, middle)) = rest.split_first() else {
            return Err("a case has no api-request".into());
        };
        if middle.is_empty() || name_of(*first) != "api-request" || name_of(*last) != "api-response"
        {
            return Err("a case does not start with api-request and end with api-response".into());
        }
        let mut exchanges = Vec::new();
        for pair in middle.chunks(2) {
            let [outgoing, incoming] = pair else {
                return Err("an outgoing message without its incoming message".into());
            };
            if name_of(*outgoing) != "outgoing-plc-message"
                || name_of(*incoming) != "incoming-plc-message"
            {
                return Err(format!(
                    "unknown step shape <{}> then <{}>",
                    name_of(*outgoing),
                    name_of(*incoming)
                ));
            }
            exchanges.push((parse_step(*outgoing, true)?, parse_step(*incoming, false)?));
        }
        Ok(Steps {
            api_request: *first,
            exchanges,
            api_response: *last,
        })
    }

    /// The tags a read request names, as `(name, address text)`.
    fn request_tags(api_request: Node<'_, '_>) -> Result<Vec<(String, String)>> {
        let tags = child(child(api_request, "TestReadRequest")?, "tags")?;
        elements(tags)
            .into_iter()
            .map(|tag| {
                let [name, address] = sequence(tag, ["name", "address"])?;
                Ok((text_of(name).to_owned(), text_of(address).to_owned()))
            })
            .collect()
    }

    /// The tag of the api-response (its decoded address) and the values it reports.
    fn response_tag(
        response: Node<'_, '_>,
        name: &str,
        address: String,
    ) -> Result<(Tag, Vec<Value>)> {
        let read = child(response, "PlcReadResponse")?;
        let tags = child(
            child(
                child(child(read, "request")?, "PlcReadRequest")?,
                "PlcTagRequest",
            )?,
            "tags",
        )?;
        let holding = child(tags, name)?
            .descendants()
            .find(|node| node.is_element() && name_of(*node) == "ModbusTagHoldingRegister")
            .ok_or_else(|| format!("tag `{name}` is not a ModbusTagHoldingRegister"))?;
        let tag = Tag {
            name: name.to_owned(),
            address,
            data_type: text_of(child(holding, "dataType")?).to_owned(),
            elements: word(child(holding, "numberOfElements")?)?,
            start: word(child(holding, "address")?)?,
        };
        let item = child(child(child(read, "values")?, name)?, "PlcResponseItem")?;
        if text_of(child(item, "code")?) != "OK" {
            return Err(format!(
                "tag `{name}`: the case expects a failure, not a value"
            ));
        }
        let value = elements(child(item, "value")?);
        let [value] = value.as_slice() else {
            return Err(format!("tag `{name}`: <value> holds more than one element"));
        };
        let singles = if name_of(*value) == "PlcList" {
            elements(*value)
        } else {
            vec![*value]
        };
        let values = singles
            .into_iter()
            .map(|single| {
                let data_type = name_of(single).strip_prefix("Plc").ok_or_else(|| {
                    format!("tag `{name}`: <{}> is not a Plc value", name_of(single))
                })?;
                Ok(Value {
                    data_type: data_type.to_owned(),
                    text: text_of(single).to_owned(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((tag, values))
    }

    /// Pair the exchanges of a read case with its tags in order and check the pairing.
    fn read_exchanges(
        name: &str,
        api_request: Node<'_, '_>,
        exchanges: Vec<(Step, Step)>,
        api_response: Node<'_, '_>,
    ) -> Result<Vec<Read>> {
        let tags = request_tags(api_request)?;
        if tags.len() != exchanges.len() {
            return Err(format!(
                "case `{name}`: {} tags but {} exchanges; only one exchange per tag is a known shape",
                tags.len(),
                exchanges.len()
            ));
        }
        let mut reads = Vec::new();
        for (index, ((tag_name, address), (request, response))) in
            tags.into_iter().zip(exchanges).enumerate()
        {
            let (tag, values) = response_tag(api_response, &tag_name, address)?;
            let quantity = usize::from(request.adu.pdu.word("quantity")?);
            let at = format!("case `{name}`, tag `{tag_name}`");
            if request.adu.pdu.function != 3
                || request.adu.pdu.word("startingAddress")? != tag.start
            {
                return Err(format!("{at}: not a function 3 read of the tag's address"));
            }
            if response.adu.pdu.value()?.len() != quantity.saturating_mul(2) {
                return Err(format!(
                    "{at}: the response does not carry {quantity} registers"
                ));
            }
            if values.len() != usize::from(tag.elements)
                || values.iter().any(|value| value.data_type != tag.data_type)
            {
                return Err(format!(
                    "{at}: the reported values are not {} x {}",
                    tag.elements, tag.data_type
                ));
            }
            reads.push(Read {
                case: name.to_owned(),
                exchange: index.saturating_add(1),
                tag,
                values,
                request,
                response,
            });
        }
        Ok(reads)
    }

    /// The reads of a case, or none for a write case (which is serialized and checked, but not
    /// emitted: writes are owned by S45 and S46).
    fn parse_case(case: Node<'_, '_>) -> Result<Option<Vec<Read>>> {
        let name = text_of(child(case, "name")?).to_owned();
        let steps = parse_steps(case).map_err(|error| format!("case `{name}`: {error}"))?;
        match elements(steps.api_request).as_slice() {
            [kind] if name_of(*kind) == "TestWriteRequest" => Ok(None),
            [kind] if name_of(*kind) == "TestReadRequest" => read_exchanges(
                &name,
                steps.api_request,
                steps.exchanges,
                steps.api_response,
            )
            .map(Some),
            _ => Err(format!("case `{name}`: unknown api-request shape")),
        }
    }

    /// The payload byte order the suite configures the driver with.
    fn payload_byte_order(root: Node<'_, '_>) -> Result<&'static str> {
        if root.attribute("byteOrder") != Some("BIG_ENDIAN") {
            return Err(
                "the harness byte order of the ADU fields is BIG_ENDIAN (modbus.mspec:22)".into(),
            );
        }
        if text_of(child(root, "driver-name")?) != "modbus-tcp" {
            return Err("the suite is not for modbus-tcp".into());
        }
        let Some(parameters) = elements(root)
            .into_iter()
            .find(|node| name_of(*node) == "driver-parameters")
        else {
            return Ok(DEFAULT_MODE);
        };
        let kids = elements(parameters);
        let [parameter] = kids.as_slice() else {
            return Err("driver-parameters holds more than default-payload-byte-order".into());
        };
        let [name, value] = sequence(*parameter, ["name", "value"])?;
        MODES
            .iter()
            .find(|mode| **mode == text_of(value) && text_of(name) == "default-payload-byte-order")
            .copied()
            .ok_or_else(|| "unknown driver parameter or byte order".to_owned())
    }

    // ---- rendering ------------------------------------------------------------------------

    enum Json {
        Number(u64),
        Text(String),
        Numbers(Vec<u64>),
        List(Vec<Self>),
        Object(Vec<(&'static str, Self)>),
    }

    /// A string that needs no escaping in JSON or TOML. Upstream names contain nothing that
    /// does, and a name that did would fail here instead of being escaped silently.
    fn plain(value: &str) -> Result<&str> {
        if value
            .chars()
            .any(|c| c == '"' || c == '\\' || c.is_control())
        {
            return Err(format!("`{value}` needs escaping"));
        }
        Ok(value)
    }

    fn count(value: usize) -> Result<Json> {
        u64::try_from(value)
            .map(Json::Number)
            .map_err(|error| error.to_string())
    }

    impl Json {
        fn text(value: &str) -> Result<Self> {
            plain(value).map(|text| Self::Text(text.to_owned()))
        }

        /// Two-space indentation, one field or list item per line, numbers inline.
        fn render(&self, depth: usize) -> String {
            let outer = "  ".repeat(depth);
            let inner = "  ".repeat(depth.saturating_add(1));
            let deeper = depth.saturating_add(1);
            match self {
                Self::Number(n) => n.to_string(),
                Self::Text(text) => format!("\"{text}\""),
                Self::Numbers(items) => {
                    let items: Vec<String> = items.iter().map(u64::to_string).collect();
                    format!("[{}]", items.join(", "))
                }
                Self::List(items) => {
                    let lines: Vec<String> = items
                        .iter()
                        .map(|item| format!("{inner}{}", item.render(deeper)))
                        .collect();
                    format!("[\n{}\n{outer}]", lines.join(",\n"))
                }
                Self::Object(fields) => {
                    let lines: Vec<String> = fields
                        .iter()
                        .map(|(key, field)| format!("{inner}\"{key}\": {}", field.render(deeper)))
                        .collect();
                    format!("{{\n{}\n{outer}}}", lines.join(",\n"))
                }
            }
        }
    }

    fn spaced_hex(bytes: &[u8]) -> String {
        let pairs: Vec<String> = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
        pairs.join(" ")
    }

    /// Lowercase ASCII words joined by `-`.
    fn slug(text: &str) -> String {
        let mut out = String::new();
        for c in text.chars() {
            if c.is_ascii_alphanumeric() {
                out.push(c.to_ascii_lowercase());
            } else if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
        }
        out.trim_end_matches('-').to_owned()
    }

    fn registers(bytes: &[u8]) -> Result<Vec<u64>> {
        let (pairs, rest) = bytes.as_chunks::<2>();
        if !rest.is_empty() {
            return Err("an odd number of register bytes".into());
        }
        Ok(pairs
            .iter()
            .map(|pair| u64::from(u16::from_be_bytes(*pair)))
            .collect())
    }

    /// The generated files of one step.
    #[derive(PartialEq)]
    struct Vector {
        /// `tcp/adu_upstream-...`, the name in `PROVENANCE.toml`.
        stem: String,
        hex: String,
        json: String,
        entry: String,
    }

    fn describe(step: &Step) -> Result<String> {
        let adu = &step.adu;
        let pdu = &adu.pdu;
        let header = format!(
            "Transaction {}, protocol id 0, length {} (unit id plus {} PDU bytes), unit id {}.",
            adu.transaction_id,
            pdu.bytes.len().saturating_add(1),
            pdu.bytes.len(),
            adu.unit_id
        );
        let detail = if pdu.request {
            format!(
                "start {}, quantity {}",
                pdu.word("startingAddress")?,
                pdu.word("quantity")?
            )
        } else {
            let value = pdu.value()?;
            format!(
                "byte count {}, register count {}",
                value.len(),
                value.len().div_euclid(2)
            )
        };
        let direction = if pdu.request { "request" } else { "response" };
        Ok(format!(
            "{header} Function 0x03 (read holding registers) {direction}: {detail}."
        ))
    }

    fn pdu_json(adu: &Adu) -> Result<Json> {
        let pdu = &adu.pdu;
        let mut fields = vec![
            (
                "direction",
                Json::text(if pdu.request { "request" } else { "response" })?,
            ),
            ("function", Json::Number(u64::from(pdu.function))),
        ];
        if pdu.request {
            fields.push((
                "start",
                Json::Number(u64::from(pdu.word("startingAddress")?)),
            ));
            fields.push(("quantity", Json::Number(u64::from(pdu.word("quantity")?))));
        } else {
            fields.push(("registers", Json::Numbers(registers(pdu.value()?)?)));
        }
        Ok(Json::Object(fields))
    }

    fn expected_json(
        read: &Read,
        suite: &Suite,
        byte_order: &str,
        request: bool,
    ) -> Result<String> {
        let step = if request {
            &read.request
        } else {
            &read.response
        };
        let adu = &step.adu;
        let frame = Json::Object(vec![
            (
                "transaction_id",
                Json::Number(u64::from(adu.transaction_id)),
            ),
            ("protocol_id", Json::Number(0)),
            ("length", count(adu.pdu.bytes.len().saturating_add(1))?),
            ("unit_id", Json::Number(u64::from(adu.unit_id))),
            ("pdu", Json::text(&spaced_hex(&adu.pdu.bytes))?),
        ]);
        let tag = Json::Object(vec![
            ("name", Json::text(&read.tag.name)?),
            ("address", Json::text(&read.tag.address)?),
            ("data_type", Json::text(&read.tag.data_type)?),
            ("elements", Json::Number(u64::from(read.tag.elements))),
        ]);
        let mut upstream = vec![
            ("file", Json::text(suite.file)?),
            ("case", Json::text(&read.case)?),
            ("step", Json::text(&step.name)?),
            ("exchange", count(read.exchange)?),
            ("byte_order", Json::text(byte_order)?),
            ("tag", tag),
        ];
        if !request {
            let values = read
                .values
                .iter()
                .map(|value| {
                    Ok(Json::Object(vec![
                        ("type", Json::text(&value.data_type)?),
                        ("text", Json::text(&value.text)?),
                    ]))
                })
                .collect::<Result<Vec<_>>>()?;
            upstream.push(("values", Json::List(values)));
        }
        let document = Json::Object(vec![
            ("adus", Json::List(vec![frame])),
            ("leftover", Json::Number(0)),
            ("pdu", pdu_json(adu)?),
            ("upstream", Json::Object(upstream)),
        ]);
        Ok(format!("{}\n", document.render(0)))
    }

    fn vector(read: &Read, suite: &Suite, byte_order: &str, request: bool) -> Result<Vector> {
        let step = if request {
            &read.request
        } else {
            &read.response
        };
        let direction = if request { "request" } else { "response" };
        let stem = format!(
            "tcp/adu_upstream-{}-{}-ex{}-{direction}",
            suite.slug,
            slug(&read.case),
            read.exchange
        );
        let (case, step_name) = (plain(&read.case)?, plain(&step.name)?);
        let hex = format!(
            "# ADU bytes: the 7 byte MBAP header (transaction id, protocol id, length, unit id), then the PDU.\n\
             # Serialized by the S04b adapter (crates/stonecrab/tests/modbus_driver_suite_adapter.rs) from the XML model of the step \"{step_name}\" of test case \"{case}\" in fixtures/modbus/upstream/tcp/{} (Apache PLC4X v1.0.0, commit {}), following modbus.mspec; Stonecrab's codec is not involved, and a test re-runs the adapter and compares this file byte for byte.\n\
             # {}\n\
             {}\n",
            suite.file,
            SHORT_COMMIT,
            describe(step)?,
            spaced_hex(&step.adu.bytes)
        );
        let entry = format!(
            "[[vector]]\nname = \"{stem}\"\nsource = \"upstream_fixture\"\norigin = \"apache/plc4x commit {COMMIT}, {UPSTREAM_DIR}/{}, test case \\\"{case}\\\", step \\\"{step_name}\\\" (kept at fixtures/modbus/upstream/tcp/{})\"\nlicense = \"Apache-2.0\"\nnote = \"Serialized from the XML model by the S04b adapter, never by Stonecrab's codec; payload byte order {byte_order}. The expected file carries the tag and the values the case reports.\"\n",
            suite.file, suite.file
        );
        Ok(Vector {
            stem,
            hex,
            json: expected_json(read, suite, byte_order, request)?,
            entry,
        })
    }

    // ---- the adapter ------------------------------------------------------------------------

    struct Summary {
        file: &'static str,
        reads: usize,
        writes: usize,
    }

    struct Output {
        vectors: Vec<Vector>,
        summaries: Vec<Summary>,
    }

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/modbus")
    }

    /// Run the adapter over the three retained files.
    fn run_adapter() -> Result<Output> {
        let mut output = Output {
            vectors: Vec::new(),
            summaries: Vec::new(),
        };
        for suite in &SUITES {
            let path = fixtures().join("upstream/tcp").join(suite.file);
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let document =
                Document::parse(&text).map_err(|error| format!("{}: {error}", suite.file))?;
            let root = document.root_element();
            let byte_order =
                payload_byte_order(root).map_err(|e| format!("{}: {e}", suite.file))?;
            let mut summary = Summary {
                file: suite.file,
                reads: 0,
                writes: 0,
            };
            for case in elements(root)
                .into_iter()
                .filter(|n| name_of(*n) == "testcase")
            {
                match parse_case(case).map_err(|error| format!("{}: {error}", suite.file))? {
                    Some(reads) => {
                        for read in &reads {
                            output.vectors.push(vector(read, suite, byte_order, true)?);
                            output.vectors.push(vector(read, suite, byte_order, false)?);
                        }
                        summary.reads = summary.reads.saturating_add(reads.len());
                    }
                    None => summary.writes = summary.writes.saturating_add(1),
                }
            }
            output.summaries.push(summary);
        }
        let stems: BTreeSet<&str> = output.vectors.iter().map(|v| v.stem.as_str()).collect();
        if stems.len() != output.vectors.len() {
            return Err("two exchanges produced the same vector name".into());
        }
        Ok(output)
    }

    /// The block of `PROVENANCE.toml` the adapter owns.
    fn provenance_block(output: &Output) -> String {
        let counts: Vec<String> = output
            .summaries
            .iter()
            .map(|summary| {
                format!(
                    "# upstream/tcp/{}: {} read exchanges serialized ({} vectors); {} write cases serialized and checked, not emitted (S45 and S46 own writes).\n",
                    summary.file,
                    summary.reads,
                    summary.reads.saturating_mul(2),
                    summary.writes
                )
            })
            .collect();
        let entries: Vec<String> = output
            .vectors
            .iter()
            .map(|vector| format!("\n{}", vector.entry))
            .collect();
        format!(
            "{BEGIN}\n# Written by crates/stonecrab/tests/modbus_driver_suite_adapter.rs and compared with this file by\n# adapter_serializes_each_driver_suite_adu_deterministically: do not edit by hand.\n{}{}{END}\n",
            counts.concat(),
            entries.concat()
        )
    }

    /// Write the vectors and the provenance block (the regeneration path).
    fn write_all(output: &Output) {
        for vector in &output.vectors {
            let base = fixtures().join(&vector.stem);
            std::fs::write(base.with_extension("hex"), &vector.hex)
                .expect("the hex file is written");
            std::fs::write(base.with_extension("expected.json"), &vector.json)
                .expect("the expected file is written");
        }
        let path = fixtures().join("PROVENANCE.toml");
        let text = std::fs::read_to_string(&path).expect("PROVENANCE.toml is readable");
        let block = provenance_block(output);
        let updated = match (text.find(BEGIN), text.find(END)) {
            (Some(start), Some(end)) => {
                let tail = text
                    .get(end.saturating_add(END.len()).saturating_add(1)..)
                    .unwrap_or_default();
                format!("{}{block}{tail}", text.get(..start).unwrap_or_default())
            }
            _ => format!("{text}\n{block}"),
        };
        std::fs::write(&path, updated).expect("PROVENANCE.toml is written");
    }

    fn committed(stem: &str, extension: &str) -> String {
        let path = fixtures().join(stem).with_extension(extension);
        std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "{}: {error}; regenerate with {REGENERATE}=1",
                path.display()
            )
        })
    }

    // covers: MB-BYTEORDER-LE, MB-BYTEORDER-BE
    #[test]
    fn adapter_serializes_each_driver_suite_adu_deterministically() {
        let first = run_adapter().expect("the three suites serialize");
        let second = run_adapter().expect("the three suites serialize again");
        assert!(
            first.vectors == second.vectors
                && provenance_block(&first) == provenance_block(&second),
            "two runs of the adapter differ"
        );
        if std::env::var_os(REGENERATE).is_some() {
            write_all(&first);
        }
        for vector in &first.vectors {
            assert_eq!(
                committed(&vector.stem, "hex"),
                vector.hex,
                "{}.hex",
                vector.stem
            );
            assert_eq!(
                committed(&vector.stem, "expected.json"),
                vector.json,
                "{}.expected.json",
                vector.stem
            );
        }
        let tcp = fixtures().join("tcp");
        let on_disk: BTreeSet<String> = std::fs::read_dir(&tcp)
            .expect("the vector directory is readable")
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| {
                SUITES
                    .iter()
                    .any(|s| name.starts_with(&format!("adu_upstream-{}-", s.slug)))
            })
            .collect();
        let emitted: BTreeSet<String> = first
            .vectors
            .iter()
            .flat_map(|v| {
                let name = v.stem.trim_start_matches("tcp/").to_owned();
                [format!("{name}.hex"), format!("{name}.expected.json")]
            })
            .collect();
        assert_eq!(
            on_disk, emitted,
            "a file is stale or missing in fixtures/modbus/tcp"
        );
        let provenance =
            std::fs::read_to_string(fixtures().join("PROVENANCE.toml")).expect("readable");
        assert!(
            provenance.contains(&provenance_block(&first)),
            "the adapter-generated block of PROVENANCE.toml is stale or was edited"
        );
        // The exchange count of each file, recorded in PROVENANCE.toml by the block above.
        let counts: Vec<(&str, usize, usize)> = first
            .summaries
            .iter()
            .map(|s| (s.file, s.reads, s.writes))
            .collect();
        assert_eq!(
            counts,
            [
                ("DriverTestsuite.xml", 4, 2),
                ("Modbus-all-datatypes.xml", 14, 14),
                ("Modbus-all-datatypes-little-endian.xml", 14, 14),
            ],
            "read exchanges and write cases per file"
        );
    }

    // ---- the serializer against upstream's own bytes ----------------------------------------

    /// The pcap-derived cases of the all-datatypes files sit inside an XML comment, so the
    /// upstream runner never loads them; they are `<raw>` bytes next to their `<xml>` model,
    /// which makes them an oracle for the serializer that does not involve Stonecrab.
    // covers: SPEC code-generation requirement 7 / independent vectors
    #[test]
    fn serializer_reproduces_the_raw_bytes_of_the_commented_pcap_cases() {
        let mut checked = 0_usize;
        for suite in &SUITES[1..] {
            let path = fixtures().join("upstream/tcp").join(suite.file);
            let text = std::fs::read_to_string(&path).expect("the retained file is readable");
            let open = text
                .find("<!--testcase>")
                .expect("the commented section starts");
            let live = text.get(..open).expect("a prefix").to_owned();
            let commented = text
                .get(open..)
                .expect("a suffix")
                .replacen("<!--testcase>", "<testcase>", 1)
                .replacen("</testcase-->", "</testcase>", 1);
            let source = format!("{live}{commented}");
            let document = Document::parse(&source).expect("the file parses");
            let pcap_cases: Vec<_> = elements(document.root_element())
                .into_iter()
                .filter(|node| name_of(*node) == "testcase" && child(*node, "raw").is_ok())
                .collect();
            assert_eq!(
                pcap_cases.len(),
                28,
                "{}: 14 queries and 14 responses",
                suite.file
            );
            for case in pcap_cases {
                let name = text_of(child(case, "name").expect("a name"));
                let raw = hex_bytes(text_of(child(case, "raw").expect("raw"))).expect("raw bytes");
                let model = child(child(case, "xml").expect("xml"), "ModbusADU").expect("a model");
                let adu = parse_adu(model, name.contains(" Query:"))
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(adu.bytes, raw, "{}: {name}", suite.file);
                checked = checked.saturating_add(1);
            }
        }
        assert_eq!(
            checked, 56,
            "28 pcap cases in each of the two all-datatypes files"
        );
    }

    // ---- loud failures ----------------------------------------------------------------------

    const REQUEST: &str = r#"<ModbusADU><ModbusTcpADU>
        <transactionIdentifier dataType="uint" bitLength="16">1</transactionIdentifier>
        <protocolIdentifier dataType="uint" bitLength="16">0</protocolIdentifier>
        <length dataType="uint" bitLength="16">6</length>
        <unitIdentifier dataType="uint" bitLength="8">1</unitIdentifier>
        <pdu><ModbusPDU>
          <errorFlag dataType="bit" bitLength="1">false</errorFlag>
          <functionFlag dataType="uint" bitLength="7">3</functionFlag>
          <ModbusPDUReadHoldingRegistersRequest>
            <startingAddress dataType="uint" bitLength="16">0</startingAddress>
            <quantity dataType="uint" bitLength="16">2</quantity>
          </ModbusPDUReadHoldingRegistersRequest>
        </ModbusPDU></pdu>
      </ModbusTcpADU></ModbusADU>"#;

    fn serialize(xml: &str, request: bool) -> Result<Vec<u8>> {
        let document = Document::parse(xml).map_err(|error| error.to_string())?;
        parse_adu(document.root_element(), request).map(|adu| adu.bytes)
    }

    #[test]
    fn a_known_model_serializes_to_the_wire_bytes_of_the_mspec() {
        assert_eq!(
            serialize(REQUEST, true).expect("serializes"),
            [0, 1, 0, 0, 0, 6, 1, 3, 0, 0, 0, 2],
            "MBAP header, function, start, quantity"
        );
    }

    // covers: SPEC code-generation requirement 2 / fail on unsupported constructs
    #[test]
    fn an_unknown_shape_or_a_disagreeing_model_fails_loudly() {
        for (from, to, reason) in [
            (">6<", ">7<", "length"),
            ("false</errorFlag>", "true</errorFlag>", "exception PDU"),
            (
                "ModbusPDUReadHoldingRegistersRequest",
                "ModbusPDUReadCoilsRequest",
                "unknown PDU",
            ),
            (r#"bitLength="16">2<"#, r#"bitLength="8">2<"#, "bitLength"),
            (
                "protocolIdentifier dataType=\"uint\" bitLength=\"16\">0<",
                "protocolIdentifier dataType=\"uint\" bitLength=\"16\">1<",
                "const",
            ),
            ("<ModbusTcpADU>", "<ModbusRtuADU>", "ModbusTcpADU"),
        ] {
            let broken = REQUEST.replace(from, to);
            let error = serialize(&broken, true).expect_err(reason);
            assert!(!error.is_empty(), "{reason}: an error is reported");
        }
        let response = REQUEST.replace(
            "ReadHoldingRegistersRequest",
            "ReadHoldingRegistersResponse",
        );
        assert!(
            serialize(&response, false).is_err(),
            "a response model needs response fields"
        );
        assert!(
            serialize(REQUEST, false).is_err(),
            "a request model is refused as a response"
        );
    }

    #[test]
    fn a_byte_count_that_disagrees_with_the_array_fails() {
        let response = r#"<ModbusADU><ModbusTcpADU>
            <transactionIdentifier dataType="uint" bitLength="16">1</transactionIdentifier>
            <protocolIdentifier dataType="uint" bitLength="16">0</protocolIdentifier>
            <length dataType="uint" bitLength="16">7</length>
            <unitIdentifier dataType="uint" bitLength="8">1</unitIdentifier>
            <pdu><ModbusPDU>
              <errorFlag dataType="bit" bitLength="1">false</errorFlag>
              <functionFlag dataType="uint" bitLength="7">3</functionFlag>
              <ModbusPDUReadHoldingRegistersResponse>
                <byteCount dataType="uint" bitLength="8">COUNT</byteCount>
                <value dataType="byte" bitLength="32">0x40490fdb</value>
              </ModbusPDUReadHoldingRegistersResponse>
            </ModbusPDU></pdu></ModbusTcpADU></ModbusADU>"#;
        assert_eq!(
            serialize(&response.replace("COUNT", "4"), false).expect("serializes"),
            [0, 1, 0, 0, 0, 7, 1, 3, 4, 0x40, 0x49, 0x0f, 0xdb],
            "byte count 4 matches the four bytes"
        );
        assert!(
            serialize(&response.replace("COUNT", "5"), false).is_err(),
            "byte count 5"
        );
    }

    #[test]
    fn a_raw_bytes_step_or_an_unknown_step_is_refused() {
        let case = |step: &str| {
            format!(
                r#"<case><name>probe</name><steps>
                <api-request name="a"><TestReadRequest><tags/></TestReadRequest></api-request>
                <{step} name="b"><raw>000100000006010300000002</raw></{step}>
                <incoming-plc-message name="c"/>
                <api-response name="d"/></steps></case>"#
            )
        };
        for (step, needle) in [
            (
                "outgoing-plc-bytes",
                "unknown step shape <outgoing-plc-bytes>",
            ),
            ("delay", "unknown step shape <delay>"),
            ("outgoing-plc-message", "holds [raw]"),
        ] {
            let text = case(step);
            let document = Document::parse(&text).expect("the snippet parses");
            let error = parse_case(document.root_element())
                .err()
                .unwrap_or_else(|| panic!("{step}: the case is refused"));
            assert!(error.contains(needle), "{step}: `{needle}` is in `{error}`");
        }
    }
}
