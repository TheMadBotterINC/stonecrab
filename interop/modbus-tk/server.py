#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# Stonecrab interoperability peer: a multi-unit Modbus TCP server on modbus-tk 1.1.5.
#
# First-party. Authored from the API of the modbus-tk 1.1.5 package (`modbus_tcp.TcpServer`,
# `Slave.add_block`, `Slave.set_values` and the `modbus.Server.on_exception` hook) and from
# the maps below. modbus-tk (LGPL-2.1-or-later) is installed from PyPI by the Dockerfile and is not
# copied here. Nothing is taken from Apache PLC4X.
#
# What it serves
#   Three units on one TCP port, each with a map of its own. A wire address is the
#   zero-based address of the request (a Stonecrab address is one more). Every value is
#   chosen so that no two units agree anywhere they overlap, so a reply that came from the
#   wrong unit is visible in the value.
#
#     unit   holding registers      input registers     coils                 discrete inputs
#     1      0..15  = 1000 + a      0..3  = 100 + a     0..7  true if a even  0..7 true if a % 3 == 0
#     2      0..15  = 2000 + a      0..3  = 200 + a     0..7  true if a odd   0..7 true if a % 3 == 1
#     247    0..3   = 24700 + a     0..1  = 2470 + a    0..2  all true        none
#
#   (a is the wire address.) Units 1 and 2 also hold two scratch holding registers, 16 and
#   17, zero at start, for the one write test; unit 247 has none. Unit 247 has the shortest
#   map on purpose: a read of holding register 8 is answered by units 1 and 2 and by unit
#   247 with exception 02 (ILLEGAL DATA ADDRESS), and so is a read of a discrete input or a
#   write of holding register 16 on unit 247.
#
#   Any other unit id, including 0 (broadcast) and 255, is not answered: modbus-tk logs
#   "Slave N doesn't exist" and sends nothing. The connection stays open and the next
#   request on it is served.
#
#   modbus-tk applies any write to the block it names. The tests write only the scratch
#   registers and put them back.
#
# Control
#   modbus-tk 1.1.5 ships an RPC simulator on port 2711 that does not run on Python 3, and
#   this script does not start it. The only control is the process: SIGTERM or SIGINT stops
#   the server, and --address and --port say where it listens. The address is explicit
#   (default 0.0.0.0): modbus-tk 1.1.5 binds every interface when it is left empty, so this
#   script never leaves it empty.
#
# Exit status: 0 after a clean stop, 1 when the server thread dies (for example when the
# port is taken).

"""Serve three Modbus units, each with its own map, over Modbus TCP."""

import argparse
import logging
import signal
import sys
import threading

from modbus_tk import defines, hooks, modbus_tcp

LOG = logging.getLogger("stonecrab.interop")

# The map of each unit: (holding registers, input registers, coils, discrete inputs), each a
# list of values indexed by wire address. An empty list means the unit has no such block.
UNITS = {
    1: (
        [1000 + a for a in range(16)] + [0, 0],
        [100 + a for a in range(4)],
        [1 if a % 2 == 0 else 0 for a in range(8)],
        [1 if a % 3 == 0 else 0 for a in range(8)],
    ),
    2: (
        [2000 + a for a in range(16)] + [0, 0],
        [200 + a for a in range(4)],
        [1 if a % 2 == 1 else 0 for a in range(8)],
        [1 if a % 3 == 1 else 0 for a in range(8)],
    ),
    247: (
        [24700 + a for a in range(4)],
        [2470 + a for a in range(2)],
        [1, 1, 1],
        [],
    ),
}

BLOCKS = (
    ("holding", defines.HOLDING_REGISTERS),
    ("input", defines.ANALOG_INPUTS),
    ("coils", defines.COILS),
    ("discrete", defines.DISCRETE_INPUTS),
)


def build_server(address, port):
    """A modbus-tk TCP server that holds the units of UNITS and is not yet started."""
    # error_on_missing_slave is the default (True): a request for an unknown unit raises
    # inside modbus-tk, is logged and gets no reply.
    server = modbus_tcp.TcpServer(port=port, address=address)
    server.set_verbose(True)
    for unit, maps in UNITS.items():
        slave = server.add_slave(unit)
        for (name, kind), values in zip(BLOCKS, maps):
            if values:
                slave.add_block(f"{name}-{unit}", kind, 0, len(values))
                slave.set_values(f"{name}-{unit}", 0, values)
    return server


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--address", default="0.0.0.0", help="address to bind (default 0.0.0.0)")
    parser.add_argument("--port", type=int, default=5030, help="TCP port (default 5030)")
    args = parser.parse_args(argv)

    logging.basicConfig(
        level=logging.DEBUG,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        stream=sys.stdout,
    )
    stop = threading.Event()
    failed = []

    def on_exception(args):
        # modbus-tk catches what ends its server thread, logs it and lets the thread end;
        # without this hook the process would wait for a signal and serve nothing.
        failed.append(args[1])
        stop.set()

    hooks.install_hook("modbus.Server.on_exception", on_exception)
    for name in (signal.SIGTERM, signal.SIGINT):
        signal.signal(name, lambda *_: stop.set())

    server = build_server(args.address, args.port)
    server.start()
    LOG.info("serving units %s on %s:%d", sorted(UNITS), args.address, args.port)
    try:
        stop.wait()
    finally:
        server.stop()
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
