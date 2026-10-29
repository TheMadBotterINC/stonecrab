# Integrations list
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State (slice S43): this is the tracked integrations list that decision E6 of
`docs/POST_ALPHA_PLAN.md` requires ("Integrations go in a tracked list and are built on
demand") and that `docs/SPEC.md` asks to be "visible in scope reporting". None of the
integrations is in the pinned baseline: the PLC4X integration modules are released as part
of the apache/plc4x-extras repository, and two of the seven live in other projects. No
integration is implemented, none is planned for a milestone, and each stays on this list
until a user asks for it. Building one starts with pinning that repository's tag and commit
in `upstream/manifest.toml`, because the baseline covers apache/plc4x only. The list is
repeated in the release matrix, `docs/COMPATIBILITY.md`, so that no integration is a hidden
omission.

Status vocabulary, as used in the Status column:

- `deferred (built on demand, E6)`: recorded in the tracked list; Stonecrab builds it when a
  user needs it, not before. The Test ids column is `none` for every row.
- `deferred (not an integration)`: the manifest looked for an integration and found only an
  example or a third-party repository; the row keeps the claim visible.
- No row is `planned`, `implemented`, `automated-verified` or `hardware-verified`, so
  `scripts/parity-check.sh` does not require these ids to appear in code.

Columns: "Upstream behavior" says where the integration lives, which release or commit the
manifest recorded and what it does; "Planned Stonecrab behavior" says what a Stonecrab
version would be. Nothing outside apache/plc4x was read beyond directory listings and a few
README files (`inventory.extras_outside_baseline`).

## File key
Citations use the manifest key and, where one exists, `file:lines` of the cited entry.

| Key | Where |
| --- | --- |
| EX (`extras_outside_baseline`) | `README.md` of apache/plc4x at the pinned commit, lines 63-77 (integrations), 92-93 (repositories) and 182-184 (examples) |
| Extras | repository apache/plc4x-extras, tag `v1.0.0`, an annotated tag object `e2749579f93bb7ae65492200fa1bff12317bd35e` that peels to commit `ed67ec04efe852e6802b6ab1a6a9fd2276956e09` (committed 2026-09-22) |

## The seven tracked integrations

| Row id | Upstream behavior (manifest key, location, release or commit) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| INTEG-CALCITE | `integration_calcite`: Apache Calcite, in Extras at `plc4j/integrations/apache-calcite` (`pom.xml`, `src`). A SQL layer over PLC tags. Source not read. | None until built. A Stonecrab version would be a separate crate, so the one-crate decision for the core library is untouched. | deferred (built on demand, E6) | none |
| INTEG-CAMEL | `integration_camel`: the `camel-plc4x` component, `Camel :: PLC4X`, in the Apache Camel repository (apache/camel) at `components/camel-plc4x` (`pom.xml`, `src`). Not in apache/plc4x or Extras; no PLC4X release tag applies and it is versioned with Camel. The default branch sets `plc4x-version` to 1.0.0 (`pom.xml:452`); latest Camel tag seen `camel-4.23.0` at `b2329103df8f5d1edfca2ad262be09e3be95ee68`. The first Camel release that depends on PLC4X 1.0.0 is UNVERIFIED. The apache/plc4x README names Camel (README.md:63-69). | None until built. | deferred (built on demand, E6) | none |
| INTEG-IOTDB | `integration_iotdb`: no IoTDB integration module exists. The only item is the example `plc4j/examples/hello-integration-iotdb` in Extras ("Application outputting PLC data into IoTDB", depends on `plc4j-api`, `iotdb-jdbc`, `iotdb-session` and `tsfile`, `pom.xml:24-86`). A code search of apache/iotdb for `plc4x` found no paths, though GitHub code search can miss files. The website claim of IoTDB support is an example application, not a shipped connector. | None until built. A write path from Stonecrab readings to IoTDB would be a new application, not a port. | deferred (built on demand, E6) | none |
| INTEG-KAFKA | `integration_kafka`: Apache Kafka Connect source and sink connectors in Extras at `plc4j/integrations/apache-kafka` (`README.md`, `config`, `pom.xml`, `src`); they pass data between Kafka and PLCs in standalone and distributed worker modes (README.md:20-36). The README still says "future 0.8 release", so it is stale. | None until built. | deferred (built on demand, E6) | none |
| INTEG-NIFI | `integration_nifi`: Apache NiFi processors in Extras at `plc4j/integrations/apache-nifi`, with separate `nifi-1` and `nifi-2` modules (`README.md`, `pom.xml`). Source not read. | None until built. | deferred (built on demand, E6) | none |
| INTEG-STREAMPIPES | `integration_streampipes`: the PLC connectors of Apache StreamPipes, in the apache/streampipes repository at `streampipes-extensions/streampipes-connectors-plc` (`pom.xml`, `src`). Not in apache/plc4x or Extras. `plc4x.version` is 0.13.1 there (`pom.xml:43, 88`), so it has not moved to 1.0.0. Latest StreamPipes tag seen `version-0.55.2` at `80fb1bc814a9aa6d5add82dd94137d640400cf6d`. | None until built. | deferred (built on demand, E6) | none |
| INTEG-DITTO | `integration_ditto`: Eclipse Ditto. There is no Ditto integration in apache/plc4x or Extras. The only match is the third-party example repository `JulianFeinauer/ditto-plc4x-example` ("Simple Example of how to use Apache PLC4X and Eclipse Ditto"), last pushed 2020-04-08; no release tag looked at. Not an Apache or Eclipse maintained module; the manifest says to ignore it for the port. | None until built. Listed because the integrations list named it; there is nothing to port. | deferred (not an integration: third-party example only) | none |

## Named by the README but not found
These items are named in the apache/plc4x README yet do not exist in the places the manifest
looked. They are listed so that the omission is visible and is not mistaken for a missed row.

| Row id | Upstream behavior (manifest key, location, release or commit) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| INTEG-KARAF | The README lists Karaf as an integration (README.md:63-77). At Extras `v1.0.0`, `plc4j/integrations` holds `apache-calcite`, `apache-kafka` and `apache-nifi` only; there is no Karaf module (`extras_outside_baseline.notes`). | None. | deferred (not found in the pinned baseline or in Extras v1.0.0) | none |
| INTEG-CAMEL-IN-EXTRAS | The README implies the Camel module is part of Extras, but there is no Camel module there; it lives in apache/camel (`INTEG-CAMEL`). | None. | deferred (not found in Extras v1.0.0; see INTEG-CAMEL) | none |

## Backlog from other languages
No `[[backlog]]` row in `upstream/manifest.toml` names an integration, so none is cited.

| Row id | Upstream behavior (manifest key, location, release or commit) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| INTEG-BACKLOG-NONE | No backlog row names an integration. | None. | deferred (no backlog row exists) | none |

## UNVERIFIED
- No integration source was read. What each one does comes from the directory listing, the
  `pom.xml` descriptions and the README lines the manifest cites.
- The first release of Camel, StreamPipes or Extras that depends on PLC4X 1.0.0 is not
  settled; StreamPipes still pins 0.13.1.
- Whether any integration exposes behavior that a Stonecrab user would expect from the core
  library (for example Kafka offsets or Calcite pushdown) is unknown until one is built.
