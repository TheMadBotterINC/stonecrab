#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# Stonecrab interoperability peer: a Modbus TCP server on pymodbus 3.8.6, and, with the
# --tls-* options, a Modbus/TCP Security server (MBAP inside TLS, slice S71, ADR-0032).
#
# First-party. Authored from two sources only: the register map
# fixtures/modbus/interop/register-map.toml (rows MB-INTEROP-*) with its sibling
# fixtures/modbus/interop/text-registers.toml (row MB-INTEROP-TEXT, slice S49), and the
# section "Interop register map" of docs/parity/modbus.md. Nothing is copied from Apache
# PLC4X, and its modbus_server.py was not consulted for this file (decision D8 of
# docs/ALPHA_PLAN.md).
#
# What it serves
#   The four data blocks (holding registers, coils, discrete inputs, input registers) are
#   filled from the map: every listed entry holds its words or bits at its logical
#   address, every other position holds zero or false. Any unit id is answered (pymodbus
#   "single" context). The blocks are writable: pymodbus applies a write to the block it
#   names and answers by its own rules (the write tests of S46 write the holding-register
#   write area 100 to 200 and the coil write area 100 to 120, and read them back).
#
# Address convention
#   The map lists logical tag addresses T; the wire address is T - 1 (the driver subtracts
#   one). pymodbus 3.8.6 adds one to the wire address before it touches a block, so a block
#   that starts at address 0 holds logical address T at list index T. Index 0 is never
#   reachable from the wire (wire address -1). The tests prove the arrangement: a read of
#   logical 13 returns the value the map lists for 13 and not its neighbour's.
#
# Block sizes
#   Registers. The map's `sizes` are read as "logical addresses 1 to N are served": a
#   register block gets N + 1 slots (slot 0 is the unreachable origin). pymodbus validates
#   FC03 and FC04 reads and accepts one when wire + 1 + count <= slots, so logical address
#   N + 1 and beyond is refused with exception code 2 (ILLEGAL DATA ADDRESS). The holding
#   block is enlarged where the map places entries beyond its size (the little-endian
#   region at 300 and 301, MB-INTEROP-LE, and the text registers from 310, MB-INTEROP-TEXT).
#
# Text registers
#   The `holding_text` entries of the text file hold their words at their logical address
#   like any other entry. Their width is the length of `words`, which the Rust test checks
#   against the type; the server does not decode text.
#   Bits. pymodbus 3.8.6 does not validate FC01 and FC02 reads at all: a read past the end
#   of a block is answered with a short or empty bit list instead of exception 2, which is
#   a malformed reply on the wire. So that every bit read the driver can send is answered
#   in full (2000 coils from logical 1, and the 2001st coil of a split batch), the coil and
#   discrete-input blocks cover the whole 16-bit address space, false where the map lists
#   nothing. The map's bit sizes (2000) therefore document the upstream server and are not
#   a limit here.
#
# TLS
#   With --tls-cert, --tls-key and --tls-ca the same blocks are served over TLS 1.2 or 1.3
#   by pymodbus's TLS server (MBAP frames inside the session) with an ssl context made
#   here: the server presents --tls-cert and --tls-key and requires a client certificate
#   issued by the authority in --tls-ca (mutual TLS). The three files are made at run time
#   by interop/pymodbus/tls.sh; none is committed. Nothing else changes.
#
# Exit status: 0 after a clean stop, 2 when the register map cannot be loaded or is
# invalid or the TLS files cannot be loaded (nothing is served in that case), 1 for any
# other failure.

"""Serve the Stonecrab interop register map over Modbus TCP."""

import argparse
import asyncio
import logging
import ssl
import sys
import tomllib

from pymodbus import FramerType
from pymodbus.datastore import (
    ModbusSequentialDataBlock,
    ModbusServerContext,
    ModbusSlaveContext,
)
from pymodbus.server import StartAsyncTcpServer, StartAsyncTlsServer

LOG = logging.getLogger("stonecrab.interop")

# Register counts of the upstream data types (docs/parity/modbus.md, "Data types").
WIDTHS = {
    "BOOL": 1, "BYTE": 1, "WORD": 1, "DWORD": 2, "LWORD": 4,
    "SINT": 1, "USINT": 1, "INT": 1, "UINT": 1, "DINT": 2, "UDINT": 2,
    "LINT": 4, "ULINT": 4, "REAL": 2, "LREAL": 4,
}  # fmt: skip

DEFAULT_MAP = "/app/register-map.toml"

DEFAULT_TEXT = "/app/text-registers.toml"

# Slots of a bit block: logical addresses 1 to 65536, plus the unreachable slot 0.
BIT_SLOTS = 65537


class MapError(Exception):
    """The register map cannot be loaded or does not describe a usable server."""


def read_toml(path):
    """Read one TOML file; raise MapError naming the problem."""
    try:
        with open(path, "rb") as handle:
            return tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise MapError(f"cannot read the register map {path}: {error}") from error


def load_map(path):
    """Read and check the register map; raise MapError naming the problem."""
    data = read_toml(path)
    sizes = data.get("sizes")
    if not isinstance(sizes, dict):
        raise MapError("the register map has no [sizes] table")
    for key in ("holding_registers", "coils", "discrete_inputs", "input_registers"):
        value = sizes.get(key)
        if not isinstance(value, int) or not 0 < value <= 65535:
            raise MapError(f"sizes.{key} must be an integer from 1 to 65535")
    return data


def place_registers(slots, entries, area, typed=True):
    """Write each entry's words at its logical address; return the slots needed.

    A typed entry must have the register count of its numeric type; an untyped one (text)
    must have at least one word."""
    needed = 0
    for entry in entries:
        kind = entry.get("type")
        words = entry.get("words")
        address = entry.get("address")
        if not isinstance(words, list) or not words:
            raise MapError(f"{area} entry {entry.get('row')}: no words")
        if typed and (kind not in WIDTHS or len(words) != WIDTHS[kind]):
            raise MapError(f"{area} entry {entry.get('row')}: type and words disagree")
        if not isinstance(address, int) or address < 1:
            raise MapError(f"{area} entry {entry.get('row')}: bad address")
        for offset, word in enumerate(words):
            if not isinstance(word, int) or not 0 <= word <= 0xFFFF:
                raise MapError(f"{area} entry {entry.get('row')}: word out of range")
            if address + offset >= len(slots):
                slots.extend([0] * (address + offset + 1 - len(slots)))
            slots[address + offset] = word
        needed = max(needed, address + len(words))
    return needed


def build_blocks(data, text):
    """Build the four pymodbus data blocks from the parsed map and text file."""
    sizes = data["sizes"]

    holding = [0] * (sizes["holding_registers"] + 1)
    place_registers(holding, data.get("holding_registers", []), "holding_registers")
    place_registers(holding, data.get("holding_extension", []), "holding_extension")
    place_registers(holding, text.get("holding_text", []), "holding_text", typed=False)

    inputs = [0] * (sizes["input_registers"] + 1)
    place_registers(inputs, data.get("input_registers", []), "input_registers")

    blocks = {"hr": holding, "ir": inputs}
    for key, short, size_key in (
        ("coils", "co", "coils"),
        ("discrete_inputs", "di", "discrete_inputs"),
    ):
        bits = [False] * max(sizes[size_key] + 1, BIT_SLOTS)
        run = data.get(key)
        if run is not None:
            address = run.get("address")
            values = run.get("values")
            if not isinstance(address, int) or address < 1 or not isinstance(values, list):
                raise MapError(f"{key}: bad run")
            if address + len(values) > len(bits):
                raise MapError(f"{key}: the run does not fit the block")
            for offset, value in enumerate(values):
                bits[address + offset] = bool(value)
        blocks[short] = bits

    return {name: ModbusSequentialDataBlock(0, values) for name, values in blocks.items()}


def trace_pdu(sending, pdu):
    """Log every request and every reply at INFO (pymodbus calls this for both)."""
    unit = getattr(pdu, "dev_id", "?")
    txn = getattr(pdu, "transaction_id", "?")
    code = getattr(pdu, "function_code", "?")
    if sending:
        LOG.info(
            "reply unit=%s txn=%s function=%s exception=%s",
            unit, txn, code, getattr(pdu, "exception_code", 0),
        )  # fmt: skip
    else:
        LOG.info(
            "request unit=%s txn=%s function=%s address=%s count=%s",
            unit, txn, code, getattr(pdu, "address", "-"), getattr(pdu, "count", "-"),
        )  # fmt: skip
    return pdu


def trace_connect(connected):
    """Log connections and disconnections."""
    LOG.info("client %s", "connected" if connected else "disconnected")


def tls_context(cert, key, authority):
    """A server context for TLS 1.2 and 1.3 that requires a client certificate."""
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.maximum_version = ssl.TLSVersion.TLSv1_3
    context.load_cert_chain(certfile=cert, keyfile=key)
    context.load_verify_locations(cafile=authority)
    context.verify_mode = ssl.CERT_REQUIRED
    return context


async def serve(blocks, host, port, tls):
    """Serve until the process is stopped; over TLS when `tls` is an ssl context."""
    context = ModbusServerContext(
        slaves=ModbusSlaveContext(
            di=blocks["di"], co=blocks["co"], hr=blocks["hr"], ir=blocks["ir"]
        ),
        single=True,
    )
    if tls is None:
        LOG.info("serving Modbus TCP on %s:%d", host, port)
        await StartAsyncTcpServer(
            context=context,
            address=(host, port),
            framer=FramerType.SOCKET,
            trace_pdu=trace_pdu,
            trace_connect=trace_connect,
        )
    else:
        LOG.info("serving Modbus TCP over TLS (mutual) on %s:%d", host, port)
        await StartAsyncTlsServer(
            context=context,
            address=(host, port),
            sslctx=tls,
            framer=FramerType.TLS,
            trace_pdu=trace_pdu,
            trace_connect=trace_connect,
        )


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--map", default=DEFAULT_MAP, help="register map (TOML)")
    parser.add_argument("--text", default=DEFAULT_TEXT, help="text registers (TOML)")
    parser.add_argument("--host", default="0.0.0.0")  # noqa: S104 (a test server)
    parser.add_argument("--port", type=int, default=5020)
    parser.add_argument("--tls-cert", help="server certificate (PEM); serve over TLS")
    parser.add_argument("--tls-key", help="server private key (PEM)")
    parser.add_argument("--tls-ca", help="authority of the client certificates (PEM)")
    args = parser.parse_args(argv)
    logging.basicConfig(
        stream=sys.stdout,
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
    )
    try:
        blocks = build_blocks(load_map(args.map), read_toml(args.text))
    except MapError as error:
        LOG.error("%s", error)
        return 2
    tls_files = (args.tls_cert, args.tls_key, args.tls_ca)
    tls = None
    if any(tls_files):
        if not all(tls_files):
            LOG.error("--tls-cert, --tls-key and --tls-ca go together")
            return 2
        try:
            tls = tls_context(*tls_files)
        except (OSError, ssl.SSLError) as error:
            LOG.error("cannot load the TLS files: %s", error)
            return 2
    try:
        asyncio.run(serve(blocks, args.host, args.port, tls))
    except KeyboardInterrupt:
        return 0
    except OSError as error:
        LOG.error("cannot serve: %s", error)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
