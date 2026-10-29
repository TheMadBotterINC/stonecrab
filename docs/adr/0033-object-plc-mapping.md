# 0033: Object PLC Mapping
## Status
Accepted (2026-10-29), as slice S72 of docs/POST_ALPHA_PLAN.md (decision E6: "PLC4X tools
count toward parity", and decision E11: "OPM by a declarative macro in this crate"). It adds
the third PLC4X tool to Stonecrab: a trait a type implements to map its fields to tags,
`PlcEntity`, with its field table `EntityField` and the field conversion `FieldValue`; a
declarative macro, `plc_entity!`, that writes the struct and that implementation; and an
`EntityManager` with its `ConnectedEntity`, built on the connection cache of ADR-0030. Every
change to the public API is additive. No Cargo feature, no dependency and no crate is added.

## Context
Upstream's `plc4j/tools/opm` (Apache PLC4X v1.0.0, commit ea229918) is a library
(`docs/parity/tools.md`, section "Object PLC Mapping (OPM)"). A class annotated `@PlcEntity`
has fields annotated `@PlcTag(value, cacheDurationMillis)`, where `value` is a tag address or
an alias `${name}` and `cacheDurationMillis` defaults to -1 (`PlcTag.java:29-33`,
`PlcEntity.java:29-32`). `PlcEntityManager` (`PlcEntityManager.java:113-249`) offers:

- `read(Class, address)`: `connect` then `disconnect` (lines 113-117).
- `write(Class, address, instance)`: `merge` then `disconnect` (lines 119-123).
- `connect(Class, address)`: generates a ByteBuddy subclass of the entity (a runtime proxy)
  with private fields for the connection string, the connection factory, the alias registry
  and two maps of timestamps, `_lastFetched` and `_lastWritten`, then reads every tagged
  field (lines 133-180, 184-210).
- `merge(Class, address, instance)`: the same proxy, with the instance's field values
  copied in, then writes every tagged field (lines 146-148, 170-175).
- `disconnect(entity)`: clears the proxy's connection factory, so its methods stop
  touching the device; a second `disconnect` throws (lines 238-254).

The proxy delegates every method to `PlcEntityInterceptor`. A getter (`getX`, or `isX`
returning a boolean) re-reads its one field if it needs syncing, then returns it
(`PlcEntityInterceptor.java:96-155, 353-395`). A setter writes its one field at once, throws
if the response code is not OK, then sets it (lines 161-200, 405-450). Any other method
first re-reads every field that needs syncing (`readAllFields`, lines 210-264). A field needs
syncing when its `cacheDurationMillis` is negative, when it was never read, or when
`now - cacheDurationMillis` is after its last read (`needsToBeSynced`, lines 331-342): so -1
means "read every time", and a field is still fresh at exactly its duration. Every request
waits at most the system property `org.apache.plc4x.java.opm.entity_manager.read_timeout`,
default 1000 ms, reads and writes alike (lines 72, 650-666). Values are converted by the
field's Java type (`getTyped`, lines 520-640): the response getters for the primitives and
their boxes, `BigInteger`, `BigDecimal`, `String`, the `java.time` types, arrays and
collections of those.

Three things in that code are worth recording because they are not ported:

- The getter path stores its timestamp under the field's short name (lines 385, 437) while
  `needsToBeSynced` looks it up under the fully qualified name that `readAllFields` uses
  (line 249). After the first expiry a getter therefore re-reads at every call.
- `writeAllFields` skips a field written within its cache duration (`lastWritten`, lines
  266-300), but `merge` always starts from an empty map, so the check never applies.
- `writeAllFields` ignores the response codes; only the setter checks them.

`AliasRegistry` (`AliasRegistry.java:19-47`) resolves an alias, optionally per connection
string. Its one implementation, `SimpleAliasRegistry` (`SimpleAliasRegistry.java:25-74`), is
a map from alias to address that ignores the connection string; `register` replaces an
existing alias, and an unknown alias throws. `OpmUtils.getOrResolveAddress`
(`OpmUtils.java:32, 52-90`) treats text that does not start with `$` as an address, text of
the form `${name}` as an alias, and refuses any other text starting with `$`.

Stonecrab is one crate (ADR-0018), so the derive macro the parity record first drew, which
needs a proc-macro crate, is ruled out; decision E11 chose a declarative macro in this
crate. Its core spawns nothing (CODING_STANDARDS 8.1, 8.7), measures time on an injected
`Clock`, and parses an address only through the driver of its connection string
(`Driver::parse_address`, decision D16). The connection cache (ADR-0030) leases one holder
at a time per connection string; the event pump (ADR-0031) already takes a lease per fetch
and gives it back.

## Decision
1. **Placement.** OPM is core: two private files, `src/entity.rs` (the trait, the field
   table, the conversions and the macro) and `src/entity_manager.rs` (the manager and the
   connected entity), re-exported at the crate root like `ConnectionCache` and
   `EventPump`. `plc_entity!` is `#[macro_export]`, so it is `stonecrab::plc_entity!`. It
   needs only core types, so it is built under every feature set, `default-features =
   false` included, and is measured with the core at 100 percent line coverage; the
   coverage and lint-attribute scripts are unchanged.
2. **No proc-macro crate** (ADR-0018, E11). The mapping is a trait a type implements, and a
   `macro_rules!` macro writes that implementation for a plain struct. Anything the macro
   cannot express (a generic struct, a computed address, a field converted by custom code)
   is written by hand against the same trait.
3. **The trait.** `PlcEntity` has three methods: `fields(&self) -> &'static [EntityField]`,
   the mapped fields in request order; `field_value(&self, name) -> Option<PlcValue>`; and
   `set_field_value(&mut self, name, PlcValue) -> Result<()>`. Fields are named, not
   indexed, so a hand implementation is a `match` on the name. The trait is object safe
   (CODING_STANDARDS 9.5); the manager's methods take it as a generic. A type may have
   fields that are not mapped. `EntityField` holds the name, the address text (or alias)
   and an optional cache duration, all `const`, so a table is a `const` slice as upstream's
   annotations are compile-time constants. The field's name is the `TagId` of its item in
   every request and response.
4. **The conversions.** `FieldValue` converts one field: `from_plc_value(PlcValue) ->
   Result<Self>` and `to_plc_value(&self) -> PlcValue`. It is implemented for `bool`, the
   eight integer types, `f32`, `f64`, `char`, `String`, `Duration`, the four calendar types
   of ADR-0021, `Vec<T>` of any of them (a `PlcValue::List`) and `PlcValue` itself.
   Reading is lenient only where nothing is lost: an integer field takes any integer
   variant whose value fits (upstream's range-checked getters), `f64` takes `F32` or `F64`,
   and every other type takes its own variant; no integer is read into a float field or a
   float into an integer one, the rule of `PlcValue::as_f64`. Writing sends the field
   type's own variant, so a written field must have its tag's type, because drivers refuse
   another one (an `InvalidConfig` item). A value a field cannot hold is
   `ErrorKind::InvalidConfig`, the kind documented as "a value incompatible with the
   requested type"; a list names the element that failed. `BigInteger` and `BigDecimal`
   have no Stonecrab value and are not ported. A user type gets a conversion by
   implementing `FieldValue`.
5. **The macro.** `plc_entity! { <attrs> <vis> struct Name { <attrs> <vis> field: Type [=>
   "address" [(cache = <const Duration>)]], ... } }` emits the struct as written, without
   the mappings, and `impl PlcEntity for Name`. A field without `=>` is not mapped, as a
   field without `@PlcTag` upstream. The mapping follows the type rather than an attribute
   because `macro_rules!` cannot pick one attribute out of a field's attribute list without
   backtracking, and a doc comment is an attribute. What does not compile, each shown by a
   `compile_fail` doctest on the macro: a mapped field whose type has no `FieldValue`
   (E0277), an address that is not a string literal (E0308), a cache duration that is not a
   constant (E0015), and a generic or tuple struct (no rule matches). The macro derives
   nothing: the struct's own attributes do. An address is checked when it is used, by the
   driver of the connection string, because the macro does not know the driver. A field's
   name is its identifier as `stringify!` writes it, so a raw identifier keeps its prefix:
   the field `r#type` is named `"r#type"`, in its `TagId` and in the trait's lookups.
6. **The manager.** `EntityManager::new(&ConnectionCache, &DriverRegistry, &dyn Clock)`
   borrows the cache, the registry the cache was built with (to parse addresses) and the
   clock the cache runs on. Each field's address is parsed by the connection string's
   driver with the connection string's address options. Every request takes a lease with
   `ConnectionCache::get`, sends one read or one write, and gives the lease back, as
   upstream's try-with-resources does with its factory, so no entity holds a connection.
   `read::<T>(cs)` (`T: Default`) is `connect` then `disconnect`; `write(cs, &T)` writes
   every mapped field in one request and returns the `WriteResponse`; `connect::<T>(cs)`
   reads every mapped field into `T::default()` and returns a `ConnectedEntity`;
   `merge(cs, T)` writes every mapped field and returns the `ConnectedEntity` with the
   response; `ConnectedEntity::disconnect(self)` gives the plain entity back. Upstream's
   default constructor is `Default`. The calendar types and `PlcValue` have no `Default`, so
   an entity with such a field implements `Default` by hand.
7. **The request timeout.** `with_request_timeout(Duration)`, default one second, puts a
   deadline that long after it is sent on every read and write, as upstream's property
   bounds both. The wait for a lease is bounded by the cache's maximum wait time instead.
   The setting is per manager; a process-wide property is not ported.
8. **Cache durations and the meaning of -1.** A field without a cache duration is
   upstream's -1: it is read again at every refresh. A field with duration `d` is fresh
   until more than `d` has passed on the clock since it was last read: at exactly `d` it
   is still fresh, as upstream's `isAfter` decides; zero means "fresh until the clock
   moves". `ConnectedEntity::refresh` reads, in one request, every field that is not fresh,
   records the clock's reading after the response for each, and resolves to how many it
   read; when every field is fresh it sends nothing. Upstream's per-getter fetch has no
   Rust counterpart (a struct's fields are read directly), so the caller asks for the
   refresh, and upstream's rule for the other methods (re-read every field that needs it,
   in one request) is the one kept. The short-name bookkeeping of the getter path is not
   reproduced. Cache durations apply to reads only, which is what upstream's behavior
   amounts to, since its write-side check never applies.
9. **Errors.** Before any I/O: an address the driver refuses, an unregistered alias or text
   that starts with `$` without being `${name}` is `InvalidAddress`, a connection string
   no driver serves is the registry's error, and an entity that lists a field it gives no
   value for is `InvalidConfig`, as is a `write` or `merge` of an entity with no mapped
   field (a write needs an item); a `read` or `connect` of such an entity sends nothing
   and gives back `T::default()`, as a refresh with every field fresh does. Every error about one field has the message
   `field "<name>": ...` and keeps the kind and diagnostics of its cause. A read whose item
   failed fails with the item's kind, before any field of a connected entity changes,
   because an entity cannot be filled without the value; a value a field cannot hold fails
   at that field, after the fields before it took their values. A write's rejected or
   unknown item is data in the `WriteResponse`, not an error (CODING_STANDARDS 6.7, 6.8),
   which also corrects upstream's `writeAllFields` ignoring its response codes.
10. **Aliases.** Upstream's `SimpleAliasRegistry` is small and is ported:
    `EntityManager::with_alias(alias, address)` registers or replaces an alias, and an
    address `${alias}` resolves through it (an empty name is a name, as upstream's `.*`
    allows). The `AliasRegistry` interface with its per-connection overloads is not ported:
    its one implementation ignores the connection, and a second implementation would add
    the trait.
11. **Read and write only.** The manager uses `Connection::read` and `Connection::write`
    through a lease and nothing else, as upstream (`TOOL-OPM-OPERATIONS`); the cache's
    `get` may open or ping the connection (ADR-0030).
12. **Not ported.** The runtime proxy (`TOOL-OPM-PROXY`): Rust has no runtime subclassing,
    and a value that performs I/O when a field is read would hide a fallible, awaited
    operation behind a field access. The setter that writes one field: a caller writes with
    `write` or `merge`. A second `disconnect` and `disconnect` of a non-entity, which cannot
    happen when `disconnect` consumes the handle. Upstream's `OPMException` wrapping: errors
    are `PlcError` with their kinds.

## Consequences
- An application describes a device as a struct and reads it with one call; the mapping is
  ordinary code it can read, test and write by hand.
- A field's type is its decoding. A wrong type is a compile error where the macro can see
  it (no `FieldValue`) and an `InvalidConfig` naming the field otherwise.
- A connected entity is only as fresh as its last `refresh`; nothing reads in the
  background. A long-lived entity costs no connection between refreshes.
- The manager borrows the cache, the registry and the clock, so it lives inside the scope
  that owns them, like the event pump.
- A per-connection alias registry, a setter that writes one field and a `FieldValue` for
  more types are additive later.

## Alternatives considered
- **A derive macro in a proc-macro crate.** Rejected by ADR-0018 and E11: a second crate to
  publish, version and audit for one attribute grammar.
- **An attribute grammar inside `macro_rules!`** (`#[tag("...")]` before the field).
  Rejected: the macro cannot tell it from the other attributes without a token muncher whose
  errors are unreadable; `=> "address"` after the type parses in one rule.
- **Fields by index instead of by name.** Rejected: a hand implementation by index is easy to
  get out of step with the table.
- **`TryFrom<PlcValue>` and `From<T> for PlcValue`.** Rejected: blanket impls of the
  standard conversion traits on `PlcValue` would apply the lenient field rules everywhere
  and conflict with the identity conversion; a trait of its own scopes them to OPM.
- **Converting each written value to its tag's type.** Rejected for now: it needs a
  conversion matrix between every pair of types, which no driver has asked for; a field
  declared with its tag's type needs none.
- **A refresh per field** (upstream's getter). Not added: one refresh of the due fields in
  one request covers it with one round trip.

## Compliance
- `crates/stonecrab/src/entity.rs` (`PlcEntity`, `EntityField`, `FieldValue`,
  `plc_entity!`) and `crates/stonecrab/src/entity_manager.rs` (`EntityManager`,
  `ConnectedEntity`), re-exported from `crates/stonecrab/src/lib.rs`.
- Tests: `crates/stonecrab/tests/core_entity_manager.rs`, on `FakeClock` against the
  simulator and against Modbus TCP over the fake transport: the macro's field table, every
  supported field type in both directions, every simulator scalar type through the manager,
  read and write in one request each, a rejected field as data, merge, a lease given back
  after every request, waited for while held and given back by a dropped request, the
  cache duration (fresh at exactly its duration, re-read after it, no duration re-read
  every time, nothing sent when all are fresh), the request timeout's default, a custom
  one and zero, aliases (resolved, replaced, unknown, malformed), an unknown address, a
  field that cannot hold its value, a failed item, a hand-written implementation, an
  entity with no mapped field, a raw identifier's name, and read and write as the only
  operations on the wire. `compile_fail` doctests on `plc_entity!`; doctests on every public item.
- `docs/parity/tools.md`: `TOOL-OPM-ENTITY`, `-MANAGER`, `-CACHE`, `-TIMEOUT`, `-ALIAS` and
  `-OPERATIONS` are implemented with their test ids; `-PROXY` stays deferred.
