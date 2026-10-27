## What

<!-- One paragraph. What changes, for whom. -->

## Why

<!-- Link the issue, the SPEC.md section, or the parity record (docs/parity/<protocol>.md#<op>). -->

## Evidence

- [ ] Unit / golden-vector / property tests added or updated: `<test names>`
- [ ] Mandatory failure cases covered (SPEC.md): `<list>` / n/a
- [ ] Mandatory-case tests carry a `// covers:` tag
- [ ] Contract suite passes against the simulator / n/a
- [ ] Interop job passes / n/a
- [ ] Hardware record linked: `docs/hardware/<file>` / no support-level claim changes

## Support level

<!-- If this PR changes any capability's status (planned -> implemented -> automated-verified -> hardware-verified), say which and link the parity record diff. -->

## Dependencies

<!-- For each new or upgraded dependency: purpose, why std is insufficient, unsafe status, license, features enabled. "None" otherwise. -->

- [ ] New `[workspace.dependencies]` entries use minor-level caret ranges (for example `"1.4"`)
- [ ] If a dependency was added: `cargo tree -e normal,dev -i once_cell -i lazy_static -i log -i time -i chrono` run and the result (empty, or each hit justified) recorded here (plan decision D23)

## Checklist

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean locally
- [ ] No `#[allow]`; every `#[expect]` has a `reason`
- [ ] Every new public item documented; `# Errors` / `# Cancel safety` / `# Wire effects` present where required
- [ ] Per-slice rustdoc sections present where applicable: `# Compatibility`, `# Errors`, `# Examples`, `# Cancel safety`, `# Wire effects`
- [ ] No new `unsafe` (there can't be, `forbid`, but confirm no dependency added without a `DEPENDENCY_AUDIT.md` entry)
- [ ] `CHANGELOG.md` `Unreleased` updated for each affected published crate
- [ ] Generated code untouched by hand; `cargo xtask generate` produces a clean diff
- [ ] Commits are Conventional Commits with `Signed-off-by`
