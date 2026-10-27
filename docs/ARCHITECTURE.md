# Stonecrab Architecture

**Status:** design document, derived from the [engineering specification](./SPEC.md). No code exists yet; every type below is a target, not a description of shipped behavior. Names are provisional until M0 pins the upstream baseline.

This document is the diagram set for the workspace. It is organized from the outside in: crate graph first, then the public contract in `stonecrab-core`, then the layers beneath it, then the dynamic behavior (state machines and sequences) that the static diagrams cannot show.

Every diagram is Mermaid so it renders on GitHub and in `cargo doc` via `#![doc = include_str!(...)]` with `rustdoc`'s `--html-in-header` mermaid shim if we choose to embed it later.

## Notation

| Mark | Meaning |
| --- | --- |
| `<<trait>>` | Rust trait. Object-safe unless noted; the public driver boundary is always object-safe. |
| `<<enumeration>>` | Rust `enum`. All public enums are `#[non_exhaustive]`. |
| `<<newtype>>` | Single-field tuple struct used for type safety (`TagId`, `UnitId`). |
| `<<generated>>` | Produced by the MSpec → Rust generator; never hand-edited. |
| `OpFuture~T~` | `Pin<Box<dyn Future<Output = Result<T, PlcError>> + Send + '_>>` — the boxed future used at the object-safe boundary. |
| `Result~T~` | `Result<T, PlcError>` unless an error type is named. |
| `A --> B` | A holds or references B. |
| `A ..> B` | A uses B transiently (argument, return, or construction). |
| `A ..|> B` | A implements trait B. |
| `A *-- B` | A owns B (composition). |

Lifetimes, `Send + Sync` bounds, and `#[derive]` lists are omitted from the diagrams; the [coding standards](./CODING_STANDARDS.md) define the mandatory bounds and derives.

---

## 1. Workspace crate graph

Dependencies point downward only: applications → facade → drivers → transport/codec → core. Sensor metadata is kept out of wire-level crates. Tooling is never a dependency of a published crate.

```mermaid
flowchart TB
    subgraph apps["Applications (outside the library boundary)"]
        APP["Vorpal, Shatterproof, third-party apps<br/>persistence · dashboards · alerting · ERP"]
        EXAMPLES["examples/"]
    end

    subgraph facade["Facade"]
        STONECRAB["<b>stonecrab</b><br/>public re-exports · driver registration · Cargo feature selection"]
    end

    subgraph drivers["Driver crates — each behind a Cargo feature"]
        SIM["<b>stonecrab-driver-sim</b><br/>simulated device + scripted failures"]
        MODBUS["<b>stonecrab-modbus</b><br/>TCP + RTU over one PDU implementation"]
        OPCUA["<b>stonecrab-opcua</b><br/>secure sessions · read · browse · native subscriptions"]
        EIP["<b>stonecrab-eip</b><br/>EtherNet/IP incl. Logix"]
        S7["<b>stonecrab-s7</b><br/>Siemens S7"]
        REST["…remaining baseline drivers<br/>ADS/AMS · SLMP · BACnet · CAN · proxy"]
    end

    subgraph sensors["Sensor layer"]
        SENSORS["<b>stonecrab-sensors</b><br/>declarative profiles · units · scaling · quality"]
        MAX["<b>stonecrab-adapter-max31855</b><br/>thermocouple over SPI"]
        ADXL["<b>stonecrab-adapter-adxl345</b><br/>accelerometer over I²C (SPI later)"]
    end

    subgraph foundation["Foundation"]
        CORE["<b>stonecrab-core</b><br/>values · tags · requests · errors · driver traits<br/><i>runtime-agnostic, no Tokio</i>"]
        TRANSPORT["<b>stonecrab-transport</b><br/>TCP · UDP · serial · deadlines · cancellation<br/><i>Tokio</i>"]
        CODEC["<b>stonecrab-codec</b><br/>bounded readers/writers · checksums · generated-message support"]
    end

    subgraph tooling["Maintainer tooling — never published, never a dependency"]
        TESTKIT["<b>stonecrab-testkit</b><br/>scripted peers · fake clock · golden vectors · contract suite · differential runner"]
        XTASK["<b>xtask</b><br/>upstream manifest · coverage · release evidence"]
        GEN["<b>stonecrab-codegen</b><br/>MSpec → Rust (maintainer-only; Java/Maven bootstrap allowed at first)"]
    end

    APP --> STONECRAB
    EXAMPLES --> STONECRAB
    APP -. "optional" .-> SENSORS

    STONECRAB --> CORE
    STONECRAB -. "feature = sim" .-> SIM
    STONECRAB -. "feature = modbus" .-> MODBUS
    STONECRAB -. "feature = opcua" .-> OPCUA
    STONECRAB -. "feature = eip" .-> EIP
    STONECRAB -. "feature = s7" .-> S7
    STONECRAB -. "feature = …" .-> REST

    SIM --> CORE
    MODBUS --> CORE
    MODBUS --> TRANSPORT
    MODBUS --> CODEC
    OPCUA --> CORE
    OPCUA --> TRANSPORT
    OPCUA --> CODEC
    EIP --> CORE
    EIP --> TRANSPORT
    EIP --> CODEC
    S7 --> CORE
    S7 --> TRANSPORT
    S7 --> CODEC
    REST --> CORE
    REST --> TRANSPORT
    REST --> CODEC

    TRANSPORT --> CORE
    CODEC --> CORE

    SENSORS --> CORE
    MAX --> SENSORS
    ADXL --> SENSORS

    TESTKIT --> CORE
    TESTKIT --> TRANSPORT
    TESTKIT --> CODEC
    TESTKIT -. "dev-dependency of" .-> MODBUS
    TESTKIT -. "dev-dependency of" .-> OPCUA
    GEN -. "emits committed code into" .-> MODBUS
    GEN -. "emits committed code into" .-> OPCUA
    XTASK -.-> GEN
    XTASK -.-> TESTKIT
```

**Rules encoded in this graph**

- `stonecrab-core` has no runtime dependency. Values, tags, requests, and errors must not change when a second async runtime is added later.
- `stonecrab-transport` is the only foundation crate that depends on Tokio. Serial, SPI, and I²C access never runs on a runtime worker thread.
- Driver crates depend on `stonecrab-transport` and `stonecrab-codec`, never on each other. Shared Modbus PDU logic lives inside `stonecrab-modbus`, not in a cross-driver crate.
- `stonecrab-sensors` depends only on `stonecrab-core`; a profile is data about tags, not about wire bytes.
- Board adapters depend on `stonecrab-sensors` so they share the measurement envelope and quality model, but they are not upstream-parity items.
- A plain `cargo add stonecrab` with default features pulls in `stonecrab-core` and nothing else. Every protocol is opt-in.

---

## 2. Mapping from Apache PLC4X concepts

Behavioral compatibility, not structural translation. This table is the index into the parity records required by the spec.

| PLC4J concept | Stonecrab type | Notes |
| --- | --- | --- |
| `PlcDriverManager` | `DriverRegistry` | Explicit registration; no classpath scanning. Unsupported scheme is a typed error. |
| `PlcDriver` | `trait Driver` | Object-safe. `connect` returns an `OpFuture`. |
| `PlcConnection` | `trait Connection` | Explicit `close`; observable `ConnectionState`. |
| `PlcConnection.getMetadata()` | `Capabilities` | Reports native vs polling subscription honestly. |
| `PlcTag` / address string | `Tag` + `dyn ProtocolAddress` | Driver parses; caller-supplied `TagId` retained end-to-end. |
| `PlcValue` hierarchy | `enum PlcValue` | Exact integer widths preserved; no f64 funnel. |
| `PlcReadRequest` / `PlcReadResponse` | `ReadRequest` / `ReadResponse` | Per-item results correlated by `TagId`; partial failure preserved. |
| `PlcWriteRequest` / `PlcWriteResponse` | `WriteRequest` / `WriteResponse` | Adds `WriteOutcome::Unknown` for ambiguous timeouts. |
| `PlcResponseCode` | `ProtocolStatus` + `ItemError` | Original protocol status kept beside the normalized error. |
| `PlcSubscriptionRequest` | `SubscribeRequest` → `Subscription` | Bounded delivery; `QueuePolicy` and `LossCounters` are mandatory. |
| `PlcBrowseRequest` | `BrowseRequest` / `BrowseResponse` | Distinct from discovery; unsupported → `ErrorKind::Unsupported`. |
| `PlcDiscoveryRequest` | `DiscoverRequest` / `DiscoveredDevice` | Never enabled by merely importing a crate. |
| `PlcConnectionException` etc. | `PlcError { kind: ErrorKind, … }` | One error type; `ErrorKind` is the discriminator. |
| `CompletableFuture<T>` | `OpFuture<T>` | Cancellation semantics documented per operation. |
| MSpec protocol definitions | `stonecrab-codegen` output in each driver | Source revision, generator version, and options recorded with output. |

---

## 3. `stonecrab-core` — public contract

The whole public API surface that applications and drivers agree on. Nothing here knows about any protocol, transport, or runtime.

### 3.1 Driver and connection boundary

```mermaid
classDiagram
    direction TB

    class DriverRegistry {
        -drivers: BTreeMap~str, Arc~dyn Driver~~
        +new() DriverRegistry
        +register(driver: Arc~dyn Driver~) Result~()~
        +driver(scheme: str) Option~Arc~dyn Driver~~
        +schemes() Vec~str~
        +parse(uri: str) Result~ConnectionConfig~
        +connect(uri: str) OpFuture~Box~dyn Connection~~
    }

    class Driver {
        <<trait>>
        +scheme() str
        +capabilities() Capabilities
        +parse_config(uri: str) Result~ConnectionConfig~
        +parse_address(address: str) Result~Box~dyn ProtocolAddress~~
        +connect(cfg: ConnectionConfig) OpFuture~Box~dyn Connection~~
    }

    class Connection {
        <<trait>>
        +state() ConnectionState
        +watch_state() StateWatch
        +capabilities() Capabilities
        +read(req: ReadRequest) OpFuture~ReadResponse~
        +write(req: WriteRequest) OpFuture~WriteResponse~
        +subscribe(req: SubscribeRequest) OpFuture~Subscription~
        +browse(req: BrowseRequest) OpFuture~BrowseResponse~
        +discover(req: DiscoverRequest) OpFuture~Vec~DiscoveredDevice~~
        +close() OpFuture~()~
    }

    class ConnectionConfig {
        +scheme: String
        +endpoint: Endpoint
        +options: BTreeMap~String, String~
        +timeouts: Timeouts
        +reconnect: ReconnectPolicy
        +read_retry: RetryPolicy
        +policy: AccessPolicy
        +limits: ResourceLimits
        +credentials: Option~Secret~Credentials~~
        +builder() ConnectionConfigBuilder
    }

    class Endpoint {
        <<enumeration>>
        Tcp(host, port)
        Udp(host, port)
        Serial(SerialSettings)
        Custom(String)
    }

    class Timeouts {
        +connect: Duration
        +request: Duration
        +idle: Option~Duration~
    }

    class ReconnectPolicy {
        +initial: Duration
        +max: Duration
        +multiplier: f64
        +jitter: Jitter
        +max_attempts: Option~u32~
        +next_delay(attempt: u32, rng) Duration
    }

    class RetryPolicy {
        +max_attempts: u32
        +retry_on: ErrorKindSet
        +note: "reads only — writes are never auto-retried"
    }

    class AccessPolicy {
        <<enumeration>>
        ReadOnly
        ReadWrite
    }

    class ResourceLimits {
        +max_frame_bytes: NonZeroUsize
        +max_buffered_bytes: NonZeroUsize
        +max_pending_requests: NonZeroUsize
        +max_queued_events: NonZeroUsize
        +max_items_per_request: Option~NonZeroUsize~
    }

    class Capabilities {
        +read: bool
        +write: bool
        +subscribe: SubscriptionMode
        +browse: bool
        +discover: bool
        +max_items_per_request: Option~NonZeroUsize~
        +max_pdu_bytes: Option~NonZeroUsize~
        +concurrency: NonZeroUsize
        +supports(op: Operation) bool
    }

    class SubscriptionMode {
        <<enumeration>>
        Unsupported
        Native
        Polling
    }

    class ConnectionState {
        <<enumeration>>
        Disconnected
        Connecting
        Connected
        Reconnecting(attempt: u32, next_at: Instant)
        Closing
        Closed
        Failed(ErrorKind)
    }

    class StateWatch {
        +current() ConnectionState
        +changed() OpFuture~ConnectionState~
    }

    class Secret~T~ {
        <<newtype>>
        -inner: T
        +expose() T
        +Debug prints REDACTED
    }

    DriverRegistry "1" o-- "*" Driver : registers
    DriverRegistry ..> ConnectionConfig : parses
    Driver ..> ConnectionConfig : accepts
    Driver ..> Connection : creates
    Driver ..> Capabilities : reports
    ConnectionConfig *-- Endpoint
    ConnectionConfig *-- Timeouts
    ConnectionConfig *-- ReconnectPolicy
    ConnectionConfig *-- RetryPolicy
    ConnectionConfig *-- AccessPolicy
    ConnectionConfig *-- ResourceLimits
    ConnectionConfig --> Secret~T~ : credentials
    Connection --> Capabilities
    Connection --> ConnectionState
    Connection --> StateWatch
    Capabilities --> SubscriptionMode
    ConnectionState ..> ErrorKind : Failed carries

    note for Driver "Object-safe. Drivers are stateless factories;\nall per-connection state lives behind Connection."
    note for Connection "Every method has documented cancel-safety.\nRequests rejected during local validation perform no I/O."
    note for RetryPolicy "Applies to reads only. Some registers have read\nside effects, so the budget is per-connection configurable."
```

### 3.2 Tags, addresses, and values

```mermaid
classDiagram
    direction TB

    class Tag {
        +id: TagId
        +address: Box~dyn ProtocolAddress~
        +value_type: Option~PlcValueType~
        +count: NonZeroU32
        +new(id, address) Tag
        +with_type(PlcValueType) Tag
    }

    class TagId {
        <<newtype>>
        -inner: Arc~str~
        +as_str() str
    }

    class ProtocolAddress {
        <<trait>>
        +protocol() str
        +canonical() String
        +value_type_hint() Option~PlcValueType~
    }

    class Any {
        <<trait>>
    }

    class PlcValue {
        <<enumeration>>
        Null
        Bool(bool)
        I8(i8)
        I16(i16)
        I32(i32)
        I64(i64)
        U8(u8)
        U16(u16)
        U32(u32)
        U64(u64)
        F32(f32)
        F64(f64)
        Char(char)
        String(String)
        Bytes(Bytes)
        Date(Date)
        Time(Time)
        DateTime(DateTime)
        Duration(Duration)
        List(Vec~PlcValue~)
        Struct(IndexMap~String, PlcValue~)
        Raw(RawValue)
        +value_type() PlcValueType
        +as_bool() Option~bool~
        +as_i64() Option~i64~
        +as_u64() Option~u64~
        +as_f64() Option~f64~
        +try_into_t~T~() Result~T, ConversionError~
    }

    class PlcValueType {
        <<enumeration>>
        Bool
        I8
        I16
        I32
        I64
        U8
        U16
        U32
        U64
        F32
        F64
        Char
        String
        Bytes
        Date
        Time
        DateTime
        Duration
        List(Box~PlcValueType~)
        Struct
        Raw
    }

    class RawValue {
        +bytes: Bytes
        +byte_order: ByteOrder
        +word_order: WordOrder
        +width_bits: u16
    }

    class ByteOrder {
        <<enumeration>>
        BigEndian
        LittleEndian
    }

    class WordOrder {
        <<enumeration>>
        HighWordFirst
        LowWordFirst
    }

    class ConversionError {
        +from: PlcValueType
        +to: PlcValueType
        +reason: ConversionReason
    }

    Tag --> TagId
    Tag --> ProtocolAddress
    Tag --> PlcValueType
    ProtocolAddress --|> Any : supertrait
    PlcValue ..> PlcValueType : value_type()
    PlcValue --> RawValue
    RawValue --> ByteOrder
    RawValue --> WordOrder
    PlcValue ..> ConversionError : try_into_t

    note for PlcValue "Exact integers are never widened to f64 implicitly.\nStruct uses an insertion-ordered map so round trips are stable."
    note for ProtocolAddress "Drivers own parsing and the concrete type. Any is a supertrait,\nso a driver downcasts its own addresses via trait upcasting (no as_any shim)."
    note for RawValue "Escape hatch for diagnosis: the bytes as received,\nwith the ordering the driver used to interpret them."
```

### 3.3 Requests, responses, and errors

```mermaid
classDiagram
    direction TB

    class ReadRequest {
        +items: Vec~Tag~
        +deadline: Option~Deadline~
        +cancel: Option~CancellationToken~
        +builder() ReadRequestBuilder
    }

    class ReadResponse {
        +items: Vec~ReadItem~
        +started_at: Instant
        +finished_at: Instant
        +split_into: NonZeroU32
        +get(id: TagId) Option~ReadItem~
        +iter_ok() Iterator
        +iter_failed() Iterator
    }

    class ReadItem {
        +tag: TagId
        +outcome: Result~PlcValue, ItemError~
        +status: Option~ProtocolStatus~
        +sampled_at: Instant
        +source_time: Option~SystemTime~
    }

    class WriteRequest {
        +items: Vec~WriteItem~
        +deadline: Option~Deadline~
        +cancel: Option~CancellationToken~
    }

    class WriteItem {
        +tag: Tag
        +value: PlcValue
    }

    class WriteResponse {
        +items: Vec~WriteResult~
        +started_at: Instant
        +finished_at: Instant
    }

    class WriteResult {
        +tag: TagId
        +outcome: WriteOutcome
        +status: Option~ProtocolStatus~
    }

    class WriteOutcome {
        <<enumeration>>
        Confirmed
        Rejected(ItemError)
        Unknown(UnknownReason)
    }

    class UnknownReason {
        <<enumeration>>
        TimeoutAfterTransmit
        DisconnectedAfterTransmit
        CancelledAfterTransmit
    }

    class SubscribeRequest {
        +items: Vec~Tag~
        +mode: SubscriptionMode
        +interval: Option~Duration~
        +queue: QueuePolicy
        +stale_after: Option~Duration~
    }

    class Subscription {
        +id: SubscriptionId
        +mode: SubscriptionMode
        +effective_interval: Option~Duration~
        +events: BoundedReceiver~SubscriptionEvent~
        +loss: LossCounters
        +unsubscribe() OpFuture~()~
    }

    class SubscriptionEvent {
        <<enumeration>>
        Value(ReadItem)
        Stale(TagId, age: Duration)
        Loss(LossSnapshot)
        StateChanged(ConnectionState)
        Ended(ErrorKind)
    }

    class QueuePolicy {
        <<enumeration>>
        Backpressure(capacity: NonZeroUsize)
        CoalesceLatest
        DropOldest(capacity: NonZeroUsize)
    }

    class LossCounters {
        +dropped: AtomicU64
        +coalesced: AtomicU64
        +missed_ticks: AtomicU64
        +snapshot() LossSnapshot
    }

    class BrowseRequest {
        +root: Option~Box~dyn ProtocolAddress~~
        +depth: Option~NonZeroU32~
        +filter: Option~BrowseFilter~
    }

    class BrowseResponse {
        +nodes: Vec~BrowseNode~
        +truncated: bool
    }

    class BrowseNode {
        +address: Box~dyn ProtocolAddress~
        +name: String
        +value_type: Option~PlcValueType~
        +readable: bool
        +writable: bool
        +subscribable: bool
        +children: Option~NonZeroU32~
    }

    class PlcError {
        +kind: ErrorKind
        +message: String
        +source: Option~Box~dyn Error~~
        +diagnostics: Diagnostics
        +new(kind, message) PlcError
        +with_source(err) PlcError
        +is_retryable() bool
    }

    class ErrorKind {
        <<enumeration>>
        InvalidConfig
        InvalidAddress
        Unsupported(Operation)
        Timeout
        Cancelled
        Disconnected
        Authentication
        Trust
        MalformedData
        DeviceException
        ResourceLimit
        Backpressure
        Internal
    }

    class ItemError {
        +kind: ErrorKind
        +status: Option~ProtocolStatus~
        +message: String
    }

    class ProtocolStatus {
        +protocol: str
        +code: u32
        +name: Option~str~
        +raw: Option~Bytes~
    }

    class Diagnostics {
        +operation: Operation
        +connection_state: ConnectionState
        +latency: Option~Duration~
        +retries: u32
        +category: FailureCategory
        +redacted: bool
    }

    class Operation {
        <<enumeration>>
        Connect
        Read
        Write
        Subscribe
        Browse
        Discover
        Close
    }

    ReadRequest "1" *-- "1..*" Tag
    ReadResponse "1" *-- "1..*" ReadItem
    ReadItem --> PlcValue
    ReadItem --> ItemError
    ReadItem --> ProtocolStatus
    WriteRequest "1" *-- "1..*" WriteItem
    WriteItem --> Tag
    WriteItem --> PlcValue
    WriteResponse "1" *-- "1..*" WriteResult
    WriteResult --> WriteOutcome
    WriteOutcome --> ItemError
    WriteOutcome --> UnknownReason
    SubscribeRequest *-- QueuePolicy
    SubscribeRequest "1" *-- "1..*" Tag
    Subscription --> SubscriptionEvent
    Subscription *-- LossCounters
    SubscriptionEvent --> ReadItem
    BrowseResponse "1" *-- "*" BrowseNode
    PlcError --> ErrorKind
    PlcError *-- Diagnostics
    ItemError --> ErrorKind
    ItemError --> ProtocolStatus
    Diagnostics --> Operation
    Diagnostics --> ConnectionState
    ErrorKind --> Operation : Unsupported carries

    note for ReadResponse "split_into > 1 means the driver split the batch to meet\ndevice limits; sampled_at differs per item and is disclosed."
    note for WriteOutcome "Unknown is a first-class result. The library never\nreplays a write to resolve it."
    note for QueuePolicy "Silent loss is prohibited. Every policy feeds LossCounters;\nBackpressure blocks the producer instead of dropping."
    note for PlcError "One error type crate-wide. Connection-level failures and\nper-tag device exceptions are never conflated."
```

### 3.4 Time and cancellation primitives

```mermaid
classDiagram
    direction LR

    class Clock {
        <<trait>>
        +now() Instant
        +wall_now() SystemTime
        +sleep_until(Instant) OpFuture~()~
    }

    class SystemClock {
        +new() SystemClock
    }

    class Deadline {
        <<newtype>>
        -at: Instant
        +after(Duration) Deadline
        +remaining(now: Instant) Option~Duration~
        +is_elapsed(now: Instant) bool
    }

    class CancellationToken {
        +new() CancellationToken
        +child() CancellationToken
        +cancel()
        +is_cancelled() bool
        +cancelled() OpFuture~()~
    }

    class StaleTracker {
        +threshold: Duration
        +last_good: Option~Instant~
        +observe_good(now: Instant)
        +observe_missed(now: Instant)
        +quality(now: Instant) Quality
    }

    SystemClock ..|> Clock
    Deadline ..> Clock : evaluated against
    StaleTracker ..> Clock : evaluated against

    note for Clock "Monotonic for deadlines and freshness; wall clock only\nfor receipt timestamps. The testkit FakeClock implements this."
    note for StaleTracker "A missed acquisition marks the retained value Stale.\nIt never republishes the previous value as a new Good reading."
```

---

## 4. `stonecrab-transport` and `stonecrab-codec`

### 4.1 Transport

```mermaid
classDiagram
    direction TB

    class Transport {
        <<trait>>
        +send(frame: Bytes, deadline: Deadline) OpFuture~()~
        +recv(buf: BytesMut, deadline: Deadline) OpFuture~usize~
        +shutdown() OpFuture~()~
        +peer() String
        +limits() ResourceLimits
    }

    class TransportFactory {
        <<trait>>
        +open(endpoint: Endpoint, limits: ResourceLimits, deadline: Deadline) OpFuture~Box~dyn Transport~~
    }

    class TcpTransport {
        -stream: TcpStream
        -limits: ResourceLimits
        +nodelay: bool
        +keepalive: Option~Duration~
    }

    class UdpTransport {
        -socket: UdpSocket
        -peer: SocketAddr
        -limits: ResourceLimits
    }

    class SerialTransport {
        -port: SerialPort
        -worker: BlockingWorker
        -settings: SerialSettings
        +inter_frame_silence() Duration
        +drain(deadline: Deadline) OpFuture~()~
    }

    class SerialSettings {
        +path: PathBuf
        +baud: u32
        +data_bits: DataBits
        +parity: Parity
        +stop_bits: StopBits
        +flow_control: FlowControl
    }

    class BlockingWorker {
        -thread: JoinHandle
        -tx: BoundedSender~SerialCommand~
        +note: "dedicated OS thread; never blocks a runtime worker"
    }

    class Framer~F~ {
        <<trait>>
        +decode(buf: BytesMut) Result~Option~F~, ParseError~
        +encode(frame: F, out: BytesMut) Result~(), ParseError~
        +max_frame_bytes() NonZeroUsize
    }

    class FramedTransport~F~ {
        -inner: Box~dyn Transport~
        -framer: Box~dyn Framer~F~~
        -rx_buf: BytesMut
        +next_frame(deadline: Deadline) OpFuture~F~
        +send_frame(frame: F, deadline: Deadline) OpFuture~()~
        +resync() OpFuture~()~
    }

    class ConnectionGuard {
        -state: ConnectionState
        -pending: Option~PendingRequest~
        -reconnect: ReconnectPolicy
        -cancel: CancellationToken
        +note: "one owner per connection; drivers wrap this"
    }

    TcpTransport ..|> Transport
    UdpTransport ..|> Transport
    SerialTransport ..|> Transport
    SerialTransport *-- SerialSettings
    SerialTransport *-- BlockingWorker
    TransportFactory ..> Transport : opens
    FramedTransport~F~ --> Transport
    FramedTransport~F~ --> Framer~F~
    ConnectionGuard --> FramedTransport~F~
    ConnectionGuard --> ReconnectPolicy
    ConnectionGuard --> CancellationToken

    note for Transport "Every call takes a Deadline. There is no unbounded I/O\nanywhere in the workspace."
    note for FramedTransport~F~ "Owns the receive buffer so TCP fragmentation and\ncoalescing are invisible to the driver."
```

### 4.2 Codec primitives and generated-message support

```mermaid
classDiagram
    direction TB

    class BoundedReader {
        -buf: Bytes
        -pos: usize
        -limit: usize
        +remaining() usize
        +read_u8() Result~u8, ParseError~
        +read_u16(ByteOrder) Result~u16, ParseError~
        +read_u32(ByteOrder) Result~u32, ParseError~
        +read_i16(ByteOrder) Result~i16, ParseError~
        +read_f32(ByteOrder) Result~f32, ParseError~
        +read_bytes(len: usize, max: usize) Result~Bytes, ParseError~
        +read_bits(count: u16) Result~BitVec, ParseError~
        +expect_eof() Result~(), ParseError~
    }

    class BoundedWriter {
        -buf: BytesMut
        -limit: usize
        +write_u8(u8) Result~(), ParseError~
        +write_u16(u16, ByteOrder) Result~(), ParseError~
        +write_bytes(Bytes) Result~(), ParseError~
        +finish() Bytes
    }

    class ParseError {
        <<enumeration>>
        Truncated(needed: usize, available: usize)
        OversizedLength(declared: usize, max: usize)
        InvalidDiscriminator(field: str, value: u64)
        InvalidChecksum(expected: u32, actual: u32)
        TrailingBytes(count: usize)
        Overflow(field: str)
        RecursionLimit(depth: u32)
    }

    class MessageCodec {
        <<trait>>
        +parse(reader: BoundedReader) Result~Self, ParseError~
        +serialize(writer: BoundedWriter) Result~(), ParseError~
        +encoded_len() usize
    }

    class Checksum {
        <<trait>>
        +update(bytes)
        +finish() u32
        +width_bytes() u8
    }

    class Crc16Modbus {
        -state: u16
    }

    class GenerationManifest {
        <<generated>>
        +upstream_tag: String
        +upstream_commit: String
        +mspec_files: Vec~PathBuf~
        +generator_version: String
        +options: BTreeMap~String, String~
        +generated_at: String
    }

    class MspecFeatureMatrix {
        +supported: BTreeSet~MspecConstruct~
        +unsupported: BTreeSet~MspecConstruct~
        +fail_on_unsupported: bool
    }

    Crc16Modbus ..|> Checksum
    MessageCodec ..> BoundedReader : parse
    MessageCodec ..> BoundedWriter : serialize
    BoundedReader ..> ParseError
    BoundedWriter ..> ParseError
    GenerationManifest --> MspecFeatureMatrix

    note for BoundedReader "Length fields are validated against a caller-supplied\nmaximum before any allocation. Panics are a bug."
    note for GenerationManifest "Emitted alongside every generated module and checked\nin CI: regenerate, then require a clean diff."
```

---

## 5. `stonecrab-modbus` — first real driver

TCP and RTU share one PDU implementation. Only framing (MBAP vs. RTU + CRC) and session rules differ.

```mermaid
classDiagram
    direction TB

    class ModbusTcpDriver {
        +scheme() "modbus-tcp"
    }

    class ModbusRtuDriver {
        +scheme() "modbus-rtu"
    }

    class ModbusConnection {
        -guard: ConnectionGuard
        -session: ModbusSession
        -batcher: Batcher
        -decoder: TypedDecoder
        -policy: AccessPolicy
    }

    class ModbusSession {
        -transaction: TransactionCounter
        -pending: Option~Pending~
        -framing: Framing
        +request(pdu: ModbusPdu, unit: UnitId, deadline: Deadline) OpFuture~ModbusPdu~
        +note: "strictly one in-flight request per RTU bus"
    }

    class Framing {
        <<enumeration>>
        Tcp(MbapFramer)
        Rtu(RtuFramer)
    }

    class MbapHeader {
        <<generated>>
        +transaction_id: u16
        +protocol_id: u16
        +length: u16
        +unit_id: u8
    }

    class MbapFramer {
        +max_adu_bytes: 260
    }

    class RtuFramer {
        +crc: Crc16Modbus
        +t15: Duration
        +t35: Duration
        +max_adu_bytes: 256
    }

    class ModbusAddress {
        +unit_id: UnitId
        +area: ModbusArea
        +offset: u16
        +count: NonZeroU16
        +convention: AddressConvention
        +parse(text: str, default_convention: AddressConvention) Result~ModbusAddress~
    }

    class ModbusArea {
        <<enumeration>>
        Coil
        DiscreteInput
        HoldingRegister
        InputRegister
    }

    class AddressConvention {
        <<enumeration>>
        ZeroBased
        OneBased40001
        Plc4xCompatible
    }

    class UnitId {
        <<newtype>>
        -inner: u8
    }

    class ModbusPdu {
        <<enumeration>>
        <<generated>>
        ReadCoils(start: u16, count: u16)
        ReadDiscreteInputs(start: u16, count: u16)
        ReadHoldingRegisters(start: u16, count: u16)
        ReadInputRegisters(start: u16, count: u16)
        WriteSingleCoil(addr: u16, value: bool)
        WriteSingleRegister(addr: u16, value: u16)
        WriteMultipleCoils(start: u16, values: BitVec)
        WriteMultipleRegisters(start: u16, values: Vec~u16~)
        ReadCoilsResponse(bits: BitVec)
        ReadRegistersResponse(regs: Vec~u16~)
        WriteResponse(addr: u16, count_or_value: u16)
        Exception(function: FunctionCode, code: ExceptionCode)
        +function_code() FunctionCode
    }

    class FunctionCode {
        <<enumeration>>
        ReadCoils = 0x01
        ReadDiscreteInputs = 0x02
        ReadHoldingRegisters = 0x03
        ReadInputRegisters = 0x04
        WriteSingleCoil = 0x05
        WriteSingleRegister = 0x06
        WriteMultipleCoils = 0x0F
        WriteMultipleRegisters = 0x10
    }

    class ExceptionCode {
        <<enumeration>>
        IllegalFunction = 0x01
        IllegalDataAddress = 0x02
        IllegalDataValue = 0x03
        ServerDeviceFailure = 0x04
        Acknowledge = 0x05
        ServerDeviceBusy = 0x06
        GatewayPathUnavailable = 0x0A
        GatewayTargetFailedToRespond = 0x0B
        Unknown(u8)
    }

    class Batcher {
        +max_registers: 125
        +max_coils: 2000
        +coalesce_contiguous: bool
        +plan(tags: Vec~Tag~) Vec~Chunk~
        +note: "preserves TagId association across chunks"
    }

    class Chunk {
        +unit_id: UnitId
        +area: ModbusArea
        +start: u16
        +count: u16
        +members: Vec~ChunkMember~
    }

    class TypedDecoder {
        +byte_order: ByteOrder
        +word_order: WordOrder
        +decode(regs: Vec~u16~, ty: PlcValueType, count: u32) Result~PlcValue, ItemError~
        +encode(value: PlcValue) Result~Vec~u16~, ItemError~
    }

    ModbusTcpDriver ..|> Driver
    ModbusRtuDriver ..|> Driver
    ModbusConnection ..|> Connection
    ModbusTcpDriver ..> ModbusConnection : creates
    ModbusRtuDriver ..> ModbusConnection : creates
    ModbusConnection *-- ModbusSession
    ModbusConnection *-- Batcher
    ModbusConnection *-- TypedDecoder
    ModbusSession *-- Framing
    Framing --> MbapFramer
    Framing --> RtuFramer
    MbapFramer ..> MbapHeader
    RtuFramer --> Crc16Modbus
    ModbusSession ..> ModbusPdu
    ModbusPdu --> FunctionCode
    ModbusPdu --> ExceptionCode
    ModbusAddress ..|> ProtocolAddress
    ModbusAddress --> ModbusArea
    ModbusAddress --> AddressConvention
    ModbusAddress --> UnitId
    Batcher ..> ModbusAddress
    Batcher ..> Chunk : produces
    ModbusConnection ..> ReadRequest
    ModbusConnection ..> ReadResponse

    note for ModbusAddress "Ambiguous 40001-vs-zero-based text is rejected unless\nthe caller or profile selects a convention explicitly."
    note for ModbusSession "A reply arriving after its deadline is discarded on TCP\n(transaction id mismatch) and drained on RTU (t3.5 silence)."
    note for TypedDecoder "Never infers word order from plausible-looking values."
```

### 5.1 Simulator driver

```mermaid
classDiagram
    direction LR

    class SimDriver {
        +scheme() "sim"
    }

    class SimConnection {
        -device: Arc~Mutex~SimDevice~~
        -script: FailureScript
        -clock: Arc~dyn Clock~
    }

    class SimDevice {
        +tags: BTreeMap~String, SimTag~
        +latency: Duration
        +max_items: Option~NonZeroUsize~
        +set(name, PlcValue)
        +ramp(name, from, to, period)
    }

    class SimTag {
        +value: PlcValue
        +readable: bool
        +writable: bool
        +read_side_effect: Option~Box~dyn Fn~~
    }

    class FailureScript {
        +steps: VecDeque~FailureStep~
        +next_for(op: Operation) Option~FailureStep~
    }

    class FailureStep {
        <<enumeration>>
        Delay(Duration)
        Timeout
        Disconnect
        DeviceException(ProtocolStatus)
        Malformed(Bytes)
        PartialFailure(Vec~TagId~)
        Backpressure
    }

    SimDriver ..|> Driver
    SimConnection ..|> Connection
    SimDriver ..> SimConnection
    SimConnection --> SimDevice
    SimConnection *-- FailureScript
    SimDevice "1" *-- "*" SimTag
    FailureScript "1" *-- "*" FailureStep

    note for SimDriver "Runs the same contract suite as every real driver.\nIt is the reference implementation of the API, not a toy."
```

---

## 6. `stonecrab-sensors` and board adapters

```mermaid
classDiagram
    direction TB

    class SensorProfile {
        +id: ProfileId
        +schema_version: SchemaVersion
        +applicability: DeviceApplicability
        +connection: ConnectionRequirements
        +measurements: Vec~MeasurementDef~
        +provenance: Provenance
        +validation: ValidationStatus
        +from_toml(text: str) Result~SensorProfile, ProfileError~
        +validate() Result~(), Vec~ProfileError~~
        +tags(driver: dyn Driver) Result~Vec~Tag~~
    }

    class DeviceApplicability {
        +manufacturer: Option~String~
        +model: Option~String~
        +firmware: Option~VersionReq~
    }

    class ConnectionRequirements {
        +scheme: String
        +default_convention: Option~AddressConvention~
        +required_options: BTreeMap~String, String~
    }

    class MeasurementDef {
        +name: String
        +address: String
        +data_type: PlcValueType
        +count: NonZeroU32
        +byte_order: Option~ByteOrder~
        +word_order: Option~WordOrder~
        +scaling: Vec~ScaleStep~
        +unit: Option~Unit~
        +range: Option~RangeCheck~
        +status_fields: Vec~StatusField~
        +quality_map: QualityMapping
        +min_interval: Duration
        +stale_after: Duration
        +keep_raw: bool
    }

    class ScaleStep {
        <<enumeration>>
        Multiply(f64)
        Divide(f64)
        Offset(f64)
        BitField(lsb: u8, width: u8, signed: bool)
        Lookup(BTreeMap~i64, f64~)
        Clamp(min: f64, max: f64)
    }

    class StatusField {
        +address: String
        +bit: Option~u8~
        +meaning: StatusMeaning
    }

    class QualityMapping {
        +good_when: Vec~Condition~
        +uncertain_when: Vec~Condition~
        +bad_when: Vec~Condition~
        +evaluate(raw, status) Quality
    }

    class Quality {
        <<enumeration>>
        Good
        Uncertain(UncertainReason)
        Bad(BadReason)
        Stale(age: Duration)
        Unknown
    }

    class Unit {
        +symbol: String
        +dimension: Dimension
    }

    class Provenance {
        +source_document: String
        +fixture_files: Vec~PathBuf~
        +license: String
    }

    class ValidationStatus {
        <<enumeration>>
        Synthetic
        Documented
        FixtureBacked
        HardwareVerified(HardwareRecordId)
    }

    class Measurement {
        +tag: TagId
        +name: String
        +value: Option~PlcValue~
        +raw: Option~PlcValue~
        +unit: Option~Unit~
        +quality: Quality
        +origin: Origin
        +received_at: SystemTime
        +received_mono: Instant
        +source_time: Option~SystemTime~
        +protocol_status: Option~ProtocolStatus~
    }

    class Origin {
        <<enumeration>>
        Protocol(scheme: str)
        Adapter(name: str)
        Derived
    }

    class Acquisition {
        -profile: Arc~SensorProfile~
        -connection: Box~dyn Connection~
        -trackers: BTreeMap~TagId, StaleTracker~
        -clock: Arc~dyn Clock~
        +plan() PollPlan
        +run(queue: QueuePolicy) OpFuture~Subscription~
        +measurements() BoundedReceiver~Measurement~
    }

    class PollPlan {
        +groups: Vec~PollGroup~
        +skip_missed_ticks: bool
    }

    class SampleBlock {
        +tag: TagId
        +interval: Duration
        +axes: AxisLayout
        +sequence: u64
        +samples: Vec~i16~
        +lost: u32
        +overflowed: bool
        +first_sample_at: Instant
    }

    SensorProfile *-- DeviceApplicability
    SensorProfile *-- ConnectionRequirements
    SensorProfile "1" *-- "1..*" MeasurementDef
    SensorProfile *-- Provenance
    SensorProfile --> ValidationStatus
    MeasurementDef "1" *-- "*" ScaleStep
    MeasurementDef "1" *-- "*" StatusField
    MeasurementDef *-- QualityMapping
    MeasurementDef --> Unit
    MeasurementDef --> PlcValueType
    QualityMapping ..> Quality : evaluate
    Measurement --> Quality
    Measurement --> Unit
    Measurement --> Origin
    Measurement --> PlcValue
    Acquisition --> SensorProfile
    Acquisition --> Connection
    Acquisition --> StaleTracker
    Acquisition ..> PollPlan
    Acquisition ..> Measurement : emits
    SampleBlock --> TagId

    note for SensorProfile "Versioned TOML. No executable code. Two devices on\nModbus are not interchangeable; profiles are per device."
    note for Quality "Good means configured checks passed. It is not a\nclaim that the physical sensor is calibrated."
    note for SampleBlock "Raw waveform data is a block with interval, layout and\nloss accounting — never a stream of unrelated scalars."
```

### 6.1 Direct bench adapters

```mermaid
classDiagram
    direction LR

    class SpiBus {
        <<trait>>
        +transfer(tx: Bytes, rx_len: usize, deadline: Deadline) OpFuture~Bytes~
    }

    class I2cBus {
        <<trait>>
        +write_read(addr: u8, tx: Bytes, rx_len: usize, deadline: Deadline) OpFuture~Bytes~
        +write(addr: u8, tx: Bytes, deadline: Deadline) OpFuture~()~
    }

    class LinuxSpiDev {
        -worker: BlockingWorker
    }

    class LinuxI2cDev {
        -worker: BlockingWorker
    }

    class Max31855 {
        -bus: Box~dyn SpiBus~
        +read(deadline: Deadline) OpFuture~ThermocoupleReading~
        +as_measurements(reading) Vec~Measurement~
    }

    class ThermocoupleReading {
        +thermocouple_c: Option~f32~
        +internal_c: f32
        +fault: Option~ThermocoupleFault~
        +raw: u32
    }

    class ThermocoupleFault {
        <<enumeration>>
        Open
        ShortToGround
        ShortToVcc
    }

    class Adxl345 {
        -bus: Box~dyn I2cBus~
        -config: Adxl345Config
        +init(deadline) OpFuture~()~
        +verify_identity(deadline) OpFuture~()~
        +read_sample(deadline) OpFuture~AccelSample~
        +read_fifo(deadline) OpFuture~SampleBlock~
    }

    class Adxl345Config {
        +range: Range
        +rate: OutputRate
        +fifo: FifoMode
        +full_resolution: bool
        +note: "initialization writes are declared config,\nnot a public actuator-write API"
    }

    class AccelSample {
        +x: i16
        +y: i16
        +z: i16
        +at: Instant
    }

    LinuxSpiDev ..|> SpiBus
    LinuxI2cDev ..|> I2cBus
    Max31855 --> SpiBus
    Max31855 ..> ThermocoupleReading
    ThermocoupleReading --> ThermocoupleFault
    Max31855 ..> Measurement : emits
    Adxl345 --> I2cBus
    Adxl345 *-- Adxl345Config
    Adxl345 ..> AccelSample
    Adxl345 ..> SampleBlock : emits

    note for Max31855 "Negative readings and the fault state are preserved.\nTests verify decode, not calibration accuracy."
    note for Adxl345 "Missing samples and FIFO overflow are reported in\nSampleBlock. Bandwidth beyond bus/host is never promised."
```

---

## 7. `stonecrab-testkit`

```mermaid
classDiagram
    direction TB

    class FakeClock {
        -now: Arc~Mutex~Instant~~
        -sleepers: BTreeMap~Instant, Vec~Waker~~
        +advance(Duration)
        +set(Instant)
    }

    class FakeTransport {
        -script: Script
        -tx_log: Vec~Bytes~
        +with_script(Script) FakeTransport
        +sent() Vec~Bytes~
    }

    class Script {
        +steps: VecDeque~Step~
        +strict: bool
    }

    class Step {
        <<enumeration>>
        Expect(Bytes)
        Reply(Bytes)
        ReplyFragmented(Vec~Bytes~)
        ReplyCoalescedWithNext
        Delay(Duration)
        CloseAt(offset: usize)
        Truncate(len: usize)
        WrongTransactionId
        LateReply(after: Duration)
    }

    class ScriptedPeer {
        -listener: TcpListener
        -script: Script
        +addr() SocketAddr
        +spawn(Script) ScriptedPeer
    }

    class VirtualSerialPair {
        +master: PathBuf
        +slave: PathBuf
        +inject_crc_error()
        +inject_gap(Duration)
    }

    class GoldenVector {
        +name: String
        +wire: Bytes
        +expected: serde_json::Value
        +source: VectorSource
        +must_fail: Option~ParseErrorKind~
    }

    class VectorSource {
        <<enumeration>>
        UpstreamFixture(path: PathBuf, rev: String)
        ProtocolSpec(section: String)
        Captured(device: String, firmware: String)
        Adversarial
    }

    class ContractSuite {
        +run(driver: Arc~dyn Driver~, cfg: ConnectionConfig) ContractReport
        +cases() Vec~ContractCase~
    }

    class ContractCase {
        +name: str
        +requires: Operation
        +run(conn: dyn Connection) CaseResult
    }

    class DifferentialRunner {
        +rust: Arc~dyn Driver~
        +upstream: UpstreamHarness
        +normalize(ReadResponse) NormalizedResponse
        +compare(fixture) DiffReport
    }

    class UpstreamHarness {
        +note: "pinned PLC4J via subprocess; maintainer-only"
    }

    class HardwareRecord {
        +device_model: String
        +firmware: String
        +adapter: String
        +wiring: String
        +host: HostInfo
        +library_commit: String
        +operations: Vec~Operation~
        +load: String
        +result: TestOutcome
        +evidence_paths: Vec~PathBuf~
    }

    FakeClock ..|> Clock
    FakeTransport ..|> Transport
    FakeTransport *-- Script
    Script "1" *-- "*" Step
    ScriptedPeer *-- Script
    GoldenVector --> VectorSource
    ContractSuite "1" *-- "*" ContractCase
    ContractSuite ..> Driver
    DifferentialRunner --> Driver
    DifferentialRunner --> UpstreamHarness

    note for ContractSuite "The same suite runs against the simulator and every\nreal driver. Capability claims are checked, not trusted."
    note for GoldenVector "Round trip alone can hide a matching encode/decode bug.\nexpected is independently authored, not derived."
```

---

## 8. Connection lifecycle

One owner per connection. The driver wraps `ConnectionGuard`, which holds exactly this state machine.

```mermaid
stateDiagram-v2
    [*] --> Disconnected

    Disconnected --> Connecting : connect()
    Connecting --> Connected : transport open + protocol handshake ok
    Connecting --> Reconnecting : I/O error, timeout (attempts remain)
    Connecting --> Failed : Authentication / Trust / InvalidConfig\nor retry budget exhausted
    Connecting --> Closed : cancel() or close()

    state Connected {
        [*] --> Idle
        Idle --> InFlight : request transmitted
        InFlight --> Idle : response correlated
        InFlight --> Resyncing : deadline elapsed
        Resyncing --> Idle : TCP: discard mismatched txn id\nRTU: drain until t3.5 silence
        Resyncing --> [*] : framing unrecoverable → force disconnect
    }

    Connected --> Reconnecting : peer close, I/O error, idle timeout
    Connected --> Closing : close()

    Reconnecting --> Connecting : backoff elapsed\n(exponential, capped, jittered)
    Reconnecting --> Failed : max_attempts reached
    Reconnecting --> Closing : close() during backoff

    Closing --> Closed : pending requests failed with Disconnected\nsubscriptions receive Ended

    Failed --> [*]
    Closed --> [*]

    note right of Reconnecting
        Pending reads fail with ErrorKind::Disconnected.
        Pending writes resolve to WriteOutcome::Unknown
        if they may have been transmitted.
    end note

    note right of Closed
        Terminal. A new connect() creates a new
        Connection; state is never reused.
    end note
```

### 8.1 Request lifecycle

Every read, write, browse and subscribe request passes through this machine. Local validation failures never reach the wire.

```mermaid
stateDiagram-v2
    [*] --> Validating
    Validating --> Rejected : InvalidAddress / Unsupported / ReadOnly policy / limits
    Validating --> Queued : ok
    Queued --> Rejected : cancel() → Cancelled\nqueue full → Backpressure
    Queued --> Transmitting : connection Idle
    Transmitting --> Awaiting : bytes flushed
    Transmitting --> Cancelled_Before_Wire : cancel() before flush
    Awaiting --> Completed : response correlated
    Awaiting --> TimedOut : deadline elapsed
    Awaiting --> Cancelled_After_Wire : cancel() after flush
    Awaiting --> Disconnected : transport closed

    TimedOut --> [*] : read → Timeout (retryable per RetryPolicy)\nwrite → Unknown(TimeoutAfterTransmit)
    Cancelled_Before_Wire --> [*] : Cancelled; device saw nothing
    Cancelled_After_Wire --> [*] : read → Cancelled\nwrite → Unknown(CancelledAfterTransmit)
    Disconnected --> [*] : read → Disconnected\nwrite → Unknown(DisconnectedAfterTransmit)
    Completed --> [*]
    Rejected --> [*] : no I/O performed
```

---

## 9. Sequences

### 9.1 Connect through the registry

```mermaid
sequenceDiagram
    autonumber
    participant App
    participant Reg as DriverRegistry
    participant Drv as ModbusTcpDriver
    participant Guard as ConnectionGuard
    participant T as TcpTransport
    participant Dev as Device

    App->>Reg: connect("modbus-tcp://10.0.0.5:502?unit-id=1")
    Reg->>Reg: scheme lookup
    alt scheme not registered
        Reg-->>App: Err(ErrorKind::Unsupported)
    end
    Reg->>Drv: parse_config(uri)
    Drv-->>Reg: ConnectionConfig
    Reg->>Drv: connect(cfg)
    Drv->>Guard: new(cfg) → state = Connecting
    Guard->>T: open(endpoint, limits, deadline = now + cfg.timeouts.connect)
    T->>Dev: TCP SYN
    Dev-->>T: SYN/ACK
    T-->>Guard: Box<dyn Transport>
    Note over Guard: Modbus TCP has no protocol handshake.<br/>OPC UA would run Hello/OpenSecureChannel/CreateSession here
    Guard->>Guard: state = Connected
    Drv-->>Reg: Box<dyn Connection>
    Reg-->>App: Box<dyn Connection>
```

### 9.2 Batched read with split and partial failure

```mermaid
sequenceDiagram
    autonumber
    participant App
    participant Conn as ModbusConnection
    participant B as Batcher
    participant S as ModbusSession
    participant Dev as Device

    App->>Conn: read(ReadRequest { items: [t1, t2, t3, t4], deadline })
    Conn->>Conn: validate addresses, policy, limits
    alt any item invalid
        Conn-->>App: Err(InvalidAddress) — no I/O
    end
    Conn->>B: plan([t1..t4])
    Note over B: t1,t2 contiguous holding regs → chunk A<br/>t3 input reg → chunk B<br/>t4 holding reg beyond 125-register window → chunk C
    B-->>Conn: [A, B, C] with TagId membership
    Conn->>S: request(ReadHoldingRegisters A, unit, deadline)
    S->>Dev: MBAP txn=7 | FC03
    Dev-->>S: MBAP txn=7 | FC03 response
    S-->>Conn: regs → decode t1, t2 (sampled_at = now₁)
    Conn->>S: request(ReadInputRegisters B, unit, deadline)
    S->>Dev: MBAP txn=8 | FC04
    Dev-->>S: MBAP txn=8 | FC84 exception 0x02
    S-->>Conn: Exception(IllegalDataAddress)
    Note over Conn: t3 → Err(ItemError { DeviceException, status: 0x02 })<br/>connection stays Connected
    Conn->>S: request(ReadHoldingRegisters C, unit, deadline)
    S->>Dev: MBAP txn=9 | FC03
    Dev-->>S: MBAP txn=9 | FC03 response
    S-->>Conn: regs → decode t4 (sampled_at = now₃)
    Conn-->>App: Ok(ReadResponse { items: [t1 Ok, t2 Ok, t3 Err, t4 Ok], split_into: 3 })
```

### 9.3 Timeout, late reply, and resynchronization

```mermaid
sequenceDiagram
    autonumber
    participant App
    participant S as ModbusSession
    participant F as FramedTransport
    participant Dev as Device

    App->>S: request(pdu, deadline = +500 ms)
    S->>F: send_frame(txn=12)
    F->>Dev: ADU txn=12
    Note over S: state InFlight(txn=12)
    S->>F: next_frame(deadline)
    F-->>S: Err(Timeout)
    S->>S: state Resyncing
    S-->>App: Err(ErrorKind::Timeout)
    Note over App: read: retry per RetryPolicy<br/>write: WriteOutcome::Unknown(TimeoutAfterTransmit)
    App->>S: request(pdu', deadline)
    S->>F: send_frame(txn=13)
    F->>Dev: ADU txn=13
    Dev-->>F: ADU txn=12 (late)
    F-->>S: frame txn=12
    S->>S: txn ≠ 13 → discard, count late_reply
    Dev-->>F: ADU txn=13
    F-->>S: frame txn=13
    S-->>App: Ok(response)
    Note over S,F: RTU variant: no transaction id, so after a timeout<br/>the session drains the port until t3.5 silence before sending again
```

### 9.4 Reconnect with backoff

```mermaid
sequenceDiagram
    autonumber
    participant App
    participant Guard as ConnectionGuard
    participant P as ReconnectPolicy
    participant T as Transport
    participant Dev as Device
    participant Sub as Subscription

    Dev--xT: FIN / RST
    T-->>Guard: recv → Err(Disconnected)
    Guard->>Guard: fail pending reads (Disconnected)<br/>pending writes → Unknown(DisconnectedAfterTransmit)
    Guard->>Sub: event StateChanged(Reconnecting{attempt:1})
    Guard->>P: next_delay(1)
    P-->>Guard: 250 ms ± jitter
    Guard->>Guard: sleep (cancellable)
    Guard->>T: open(endpoint, deadline)
    T--xDev: connect refused
    Guard->>P: next_delay(2)
    P-->>Guard: 500 ms ± jitter
    Guard->>Guard: sleep (cancellable)
    Guard->>T: open(endpoint, deadline)
    T->>Dev: TCP SYN
    Dev-->>T: SYN/ACK
    Guard->>Guard: state = Connected
    Guard->>Sub: event StateChanged(Connected)
    Note over Guard,Sub: Polling subscriptions resume on the next tick.<br/>Native subscriptions (OPC UA) are re-created and reported.
    opt max_attempts reached
        Guard->>Guard: state = Failed(Disconnected)
        Guard->>Sub: event Ended(Disconnected)
        Guard-->>App: watch_state() yields Failed
    end
```

### 9.5 Polling subscription with coalescing, loss accounting, and stale marking

```mermaid
sequenceDiagram
    autonumber
    participant App
    participant Sub as Subscription
    participant Poll as PollTask
    participant Track as StaleTracker
    participant Conn as Connection
    participant Clk as Clock

    App->>Conn: subscribe(items, mode: Polling, interval: 1 s, queue: CoalesceLatest, stale_after: 3 s)
    Conn-->>App: Subscription { effective_interval: 1 s, mode: Polling }
    loop every tick
        Poll->>Clk: now()
        Poll->>Conn: read(items, deadline = tick + interval)
        alt read ok
            Conn-->>Poll: ReadResponse
            Poll->>Track: observe_good(now)
            Poll->>Sub: Value(item) — coalesce if consumer is behind
            Note over Sub: coalesced counter += 1 when a value is replaced
        else timeout / device exception
            Conn-->>Poll: Err / Item Err
            Poll->>Track: observe_missed(now)
            Poll->>Track: quality(now)
            alt age ≥ stale_after
                Track-->>Poll: Stale(age)
                Poll->>Sub: Stale(tag, age)
            else
                Track-->>Poll: last quality retained, not re-emitted
            end
        end
        opt read overran the interval
            Note over Poll: skip missed ticks (missed_ticks += n)<br/>never flood a recovering device
        end
    end
    App->>Sub: unsubscribe()
    Sub->>Poll: cancel
    Sub-->>App: Ok(()), final LossSnapshot available
```

### 9.6 Write with an ambiguous outcome

```mermaid
sequenceDiagram
    autonumber
    participant App
    participant Conn as Connection
    participant S as Session
    participant Dev as Device

    App->>Conn: write(WriteRequest { items: [setpoint = 42] })
    Conn->>Conn: policy == ReadWrite? else Err(Unsupported) with no I/O
    Conn->>Conn: encode value via TypedDecoder (word order from tag/profile)
    Conn->>S: request(WriteSingleRegister, deadline)
    S->>Dev: FC06
    Note over Dev: device applies the write…
    Dev--xS: reply lost
    S-->>Conn: Err(Timeout) after transmit
    Conn-->>App: WriteResponse { items: [ Unknown(TimeoutAfterTransmit) ] }
    Note over App: The library never replays the write.<br/>The application decides: read back and compare, or escalate.
```

### 9.7 Sensor profile → measurement pipeline

```mermaid
sequenceDiagram
    autonumber
    participant App
    participant Prof as SensorProfile
    participant Drv as Driver
    participant Acq as Acquisition
    participant Conn as Connection
    participant Scale as ScaleStep chain
    participant QM as QualityMapping

    App->>Prof: from_toml("profiles/acme-tx100.toml")
    Prof->>Prof: validate(): overlapping bit fields, counts, conventions, schema version
    alt invalid
        Prof-->>App: Err(Vec<ProfileError>)
    end
    App->>Prof: tags(driver)
    Prof->>Drv: parse_address("holding:0x0010", convention: ZeroBased)
    Drv-->>Prof: ModbusAddress
    Prof-->>App: Vec<Tag> (TagId = measurement name)
    App->>Acq: new(profile, connection, clock)
    App->>Acq: run(queue: Backpressure(64))
    loop per poll group
        Acq->>Conn: read(group tags)
        Conn-->>Acq: ReadResponse
        Acq->>Scale: apply in declared order (Multiply → Offset → Clamp)
        Scale-->>Acq: scaled f64 or exact integer (non-finite → Bad)
        Acq->>QM: evaluate(raw, status_fields)
        QM-->>Acq: Quality
        Acq-->>App: Measurement { value, raw (if keep_raw), unit, quality, received_at, source_time: None }
    end
```

---

## 10. Cargo feature matrix (planned)

| Crate | Feature | Pulls in | Default |
| --- | --- | --- | --- |
| `stonecrab` | `sim` | `stonecrab-driver-sim` | no |
| `stonecrab` | `modbus` | `stonecrab-modbus` (TCP + RTU) | no |
| `stonecrab` | `modbus-rtu` | `stonecrab-modbus/serial` → `stonecrab-transport/serial` | no |
| `stonecrab` | `opcua` | `stonecrab-opcua` | no |
| `stonecrab` | `eip`, `s7`, `ads`, `slmp`, … | respective driver crates | no |
| `stonecrab` | `sensors` | `stonecrab-sensors` | no |
| `stonecrab` | `adapters-linux` | both board adapters with Linux `spidev`/`i2c-dev` backends | no |
| `stonecrab` | `full` | every published driver and sensor crate | no |
| `stonecrab-transport` | `tcp`, `udp`, `serial` | Tokio net / `serialport` + blocking worker | `tcp` |
| `stonecrab-sensors` | `toml` | profile loading from TOML | yes |

Features are additive. No feature may remove an API or change a default behavior. Enabling a feature never opens a network endpoint or starts hardware discovery.

---

## 11. What this document does not decide

- Exact MSpec constructs supported at M1 (decided in M0 inventory).
- Whether `PlcValue::Date/Time/DateTime` are `std::time::SystemTime`-based or a dedicated calendar type (decided by an ADR before M1; `chrono`/`time`/`jiff` are not permitted in `stonecrab-core` by default).
- OPC UA session and security-channel class structure (M3 design doc).
- The C ABI, dynamic loading, and `no_std` subsets (post-M5 extensions).
