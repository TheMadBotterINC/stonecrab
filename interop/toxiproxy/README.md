# Stonecrab failure-path interoperability run

The failure paths of the Modbus TCP driver on a real clock (slice S52b): the two interop
peers, pymodbus 3.8.6 and libmodbus 3.1.11, behind [Toxiproxy](https://github.com/Shopify/toxiproxy)
2.13.0, plus `docker pause`, `docker kill` and `docker start` on the pymodbus container, and an
independent decode of the whole run with tshark. The `interop-toxiproxy` job of
`.github/workflows/ci.yml` and `mise run interop-toxiproxy` (also part of `mise run interop`)
build it, run the tests and keep the evidence.

## First-party statement and provenance

`interop.sh`, `toxiproxy.json` and `tshark/Dockerfile` are first-party and Apache-2.0
(`PROVENANCE.toml` lists them). The tools are not: Toxiproxy (MIT) is pulled as
`ghcr.io/shopify/toxiproxy:2.13.0` by the index digest in `interop.sh`, and tshark (Wireshark,
GPL-2.0-or-later) is installed from Debian trixie into an image built from `tshark/Dockerfile`
on the `debian:trixie-slim` digest that `interop/libmodbus/Dockerfile` uses. Nothing of either
is copied into the repository. The tshark package is not pinned by version, because Debian
drops a superseded security release from the archive; the version that ran is written to
`image-digest.txt` in the evidence. To move Toxiproxy, change the tag and digest in
`interop.sh`, pull, and record them here and in `PROVENANCE.toml`.

## What runs

Everything is on one Docker network, `stonecrab-toxi`: the two peers (no published port) and
Toxiproxy, whose proxies `pymodbus` (container port 15020, upstream `stonecrab-toxi-pymodbus:5020`)
and `libmodbus` (15021, upstream `stonecrab-toxi-libmodbus:1502`) are in `toxiproxy.json`. The
host reaches Toxiproxy on `127.0.0.1` only: the API on 18474 and the proxies on 15020 and 15021
(each overridable, see `interop.sh`). The API has no authentication, so it must not be
published beyond the loopback; inside the network only the three containers of the run can
reach it.

The tests (`crates/stonecrab/tests/modbus_interop_toxiproxy.rs`, ten of them, one at a time)
are the controller. Over the HTTP API they add a toxic to a proxy while a request is in
flight, remove it, and enable or disable the proxy, using a small client on `std::net`
that needs no dependency. A script around separate runs could not time a toxic to a request.
They run `docker pause`, `unpause`, `kill` and `start` on the pymodbus container themselves.

| Fault | Toxic or command | Outcome the test asserts |
| --- | --- | --- |
| A reset on the path | `reset_peer`, each direction, both peers | `Disconnected` at once; the next request reconnects |
| A black hole | `timeout` with no timeout, each direction, both peers | `Timeout` at the 500 ms deadline, twice; after the toxic goes, the link recovers (Toxiproxy closes it) |
| Latency below the deadline | `latency` 300 ms, jitter 100 ms | the read succeeds in 200 ms to 2 s |
| Latency above the deadline | `latency` 1500 ms, jitter 200 ms | `Timeout` at 500 ms; the delayed reply is discarded and the next read succeeds |
| Sliced frames | `slicer`, both directions, about 3 bytes | ten registers read the same sliced and whole |
| A link closed after N bytes | `limit_data` of one whole reply | the first read succeeds, the next is `Disconnected` |
| A frame cut inside its header | `limit_data` of 4 bytes, each direction | `Disconnected`; recovery follows |
| Every connection closed | the proxy disabled, then enabled | `Disconnected` on two open pymodbus connections and one libmodbus connection; reconnect after enabling |
| A hung peer | `docker pause` | `Timeout` at the deadline; after `unpause` the late reply is discarded and the same connection answers |
| A dead peer | `docker kill`, then `docker start` | `Disconnected`, also while the peer is down; reads again once it is back |

Recovery allows `Disconnected` and no other error kind, a few times, because a link that the
proxy closed is found by the request that follows. Each test begins and ends with the API's
`/reset`, and the two tests that touch the container put it back whatever happens.

## The capture

`interop.sh capture` starts tshark in the network namespace of the Toxiproxy container,
which carries both legs of every connection (client to proxy, proxy to peer). `interop.sh check`
stops it, keeps the pcap, and decodes it in the tshark container with the four ports declared
as Modbus/TCP. It fails on:

- any malformed packet or expert-info error;
- a segment sent to a proxy that is not one Modbus/TCP frame with protocol id 0 and a length
  field that matches the segment, or a read or single write whose length is not 6;
- a segment sent by a peer that is not Modbus/TCP with protocol id 0;
- a capture that has no Modbus/TCP frame on any of the four legs.

What it does not do: the Wireshark dissector is lenient (it accepts a read request whose
PDU is short if the length field agrees) and does not classify requests and replies on ports
other than 502, so the check is on the header and the lengths, not a full second parse of
the PDUs. The truncated frames that `limit_data` makes on the proxy-to-client leg are not
counted against the driver, which did not send them. The checks were tried on a capture of
deliberately bad frames (protocol id 1, a wrong length, a short PDU), and each is flagged.

## Build and run

From the repository root, with Docker running:

```sh
mise run interop-toxiproxy
```

or the steps one by one, as the CI job does: `interop/toxiproxy/interop.sh build`, `start`,
`wait`, `capture`, `tests`, `check`, `evidence`, `stop`. The evidence is written to
`target/interop-evidence/toxiproxy/`: `image-digest.txt` (image ids, the Toxiproxy and tshark
versions), `nextest.txt`, `toxiproxy.pcap`, `capture-check.txt` (the count of frames each check
selected), `capture.log` and the logs of the three containers.
