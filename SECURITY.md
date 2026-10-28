# Security

## Status
Stonecrab is alpha software. The alpha supports reading coils, discrete inputs, holding
registers and input registers from Modbus TCP servers, an in-process simulator and
declarative sensor profiles. It does not write to devices, subscribe, browse, discover or
speak Modbus RTU, and it has been checked against one independent server (pymodbus 3.8.6)
and no physical device. See [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md) for the exact
support levels. The only published version is `stonecrab` 0.1.0, a specification-stage
package with no protocol code; `stonecrab` 0.2.0, the alpha, is one crate and is
unreleased: it is published only after a separate maintainer approval.

## Supported versions
The alpha line is the only supported line. Nothing older is supported, and the 0.1.0
placeholder has no code to defend.

| Crate | Supported version |
| --- | --- |
| `stonecrab` | 0.2.0 (once published) |

Stonecrab is one crate with one version; its modules (the simulator, Modbus TCP, the
sensor profiles, the transport and the codec) are not versioned separately. Pre-1.0
releases may change their API between minor versions. A fix for a reported defect, in any
module, ships as a patch release of `stonecrab`.

## Reporting a vulnerability
Do not report vulnerabilities with credentials or private device data in public issues.
Use GitHub private vulnerability reporting on the repository when enabled. If that
channel is unavailable, contact the maintainers privately before sending details.
Enabling and verifying that channel is a release prerequisite.

The response target is acknowledgement within three business days, with a coordinated
disclosure window of 90 days or a shorter period agreed with the reporter.

## Yanking and advisories
A published version with a confirmed soundness or safety defect is yanked from crates.io
within 24 hours of confirmation. Defects in this class include unbounded allocation, a panic
on input and write replay (re-sending a write whose outcome is unknown). The same
confirmation files a RUSTSEC advisory, and a fixed patch release of `stonecrab` follows.
There is one crate, so one `cargo yank` covers a version and there is no sibling to yank
with it. This is the rule of `docs/CODING_STANDARDS.md` section 20.5. Yanking does not
remove a version from a lock file that already holds it, so the advisory states the
affected and the fixed versions. The mechanics (the `cargo yank` command, the advisory
pull request, the patch release) are written down in
[`docs/adr/0017-release-process-and-evidence.md`](docs/adr/0017-release-process-and-evidence.md),
decision 5.

Because nothing beyond the 0.1.0 placeholder is published, no version has been yanked and
no advisory has been filed.
