# opcua-crypto-probe

Slice S82 of docs/POST_ALPHA_PLAN.md: evidence for decision E15 (the cryptography dependency of
the OPC UA security policies). An out-of-tree spike on the unmerged branch
`spike/s82-crypto-probe`; it is its own Cargo workspace, no Stonecrab crate depends on it, and
it is never published. The tests exercise `aws-lc-rs` 1.19.0 (default features off, `aws-lc-sys`
and `alloc`), `aws-lc-sys` 0.47.0 with `disable-prebuilt-nasm`, `rustls-webpki` 0.103.16 with no
crypto provider feature and `rustls-pki-types` 1.15.1. `.github/workflows/s82-crypto-probe.yml`
runs them on the CI matrix.
