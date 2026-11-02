#!/usr/bin/env python3
"""Rebuild encoder tables from minimp3's CC0 decoder lookups using Python 3.

Run with --check to verify, or --write to regenerate src/huffman_tables.rs.
The pinned upstream file is fetched into memory, never saved locally. For
offline use, --source accepts an existing minimp3.h (kept outside the repo).
Both modes refuse to change any existing codeword or length.
"""

import argparse
import hashlib
from pathlib import Path
import re
import urllib.request


COMMIT = "ea99364f61c14656440e8d77e9c233ccf3124633"
URL = f"https://raw.githubusercontent.com/lieff/minimp3/{COMMIT}/minimp3.h"
SHA256 = "57e437c5c1f0e8b243885d3929c8973b5e6c778451e0100ab4251d19915cb3ad"
TARGET = Path(__file__).resolve().parents[1] / "src/huffman_tables.rs"
WIDTHS = {
    1: 2, 2: 3, 3: 3, 5: 4, 6: 4, 7: 6, 8: 6, 9: 6,
    10: 8, 11: 8, 12: 8, 13: 16, 15: 16, 16: 16, 24: 16,
}
HEADER = f"""//! Huffman code tables (ISO/IEC 11172-3, Table B.7).
//!
//! Generated from minimp3's CC0 minimp3.h, L3_huffman: tabs, tabindex and tab32.
//! Source: https://github.com/lieff/minimp3/blob/{COMMIT}/minimp3.h
//! Regenerate: python3 tools/generate_huffman_tables.py --write
//! Verify: python3 tools/generate_huffman_tables.py --check
//! The generator walks decoder lookups back to codewords, excluding sign/linbits.
//!
//! Pair tables are indexed by `x * width + y`; quad tables by `v << 3 | w << 2 | x << 1 | y`.
"""


def c_array(source, name):
    body = re.search(r"\b" + name + r"\[[^]]*\]\s*=\s*\{([^}]+)\}", source)[1]
    return [int(n) for n in re.findall(r"-?\d+", body)]


def remember(found, symbol, code, length):
    value = (code, length)
    # Short leaves are repeated for every possible unused lookahead suffix.
    assert symbol not in found or found[symbol] == value, (symbol, value)
    assert 0 <= code < (1 << length)
    found[symbol] = value


def pair_table(tabs, root, width):
    found = {}

    def walk(offset, lookahead, prefix, consumed):
        for index in range(1 << lookahead):
            leaf = tabs[root + offset + index]
            if leaf < 0:
                # Pointer offsets are relative to this codebook's root, not
                # the current lookup. Python's >> is arithmetic, as in C here.
                walk(-(leaf >> 3), leaf & 7,
                     (prefix << lookahead) | index, consumed + lookahead)
            else:
                used = leaf >> 8  # Bits consumed in the final lookup only.
                assert 0 < used <= lookahead
                x, y = leaf & 15, (leaf >> 4) & 15
                assert x < width and y < width
                code = (prefix << used) | (index >> (lookahead - used))
                remember(found, x * width + y, code, consumed + used)

    walk(0, 5, 0, 0)
    assert set(found) == set(range(width * width))
    return [found[i] for i in range(width * width)]


def quad_table(lookup):
    found = {}
    for prefix in range(16):
        leaf = lookup[prefix]
        # Bit 3 marks a direct leaf; otherwise bits 0..1 give the next
        # lookup width and bits 3..7 its offset. Leaf lengths are total lengths.
        extra = 0 if leaf & 8 else leaf & 3
        for suffix in range(1 << extra):
            terminal = leaf if leaf & 8 else lookup[(leaf >> 3) + suffix]
            length = terminal & 7
            assert 0 < length <= 4 + extra
            code = ((prefix << extra) | suffix) >> (4 + extra - length)
            remember(found, terminal >> 4, code, length)
    assert set(found) == set(range(16))
    return [found[i] for i in range(16)]


def generate(source):
    tabs, offsets = c_array(source, "tabs"), c_array(source, "tabindex")
    arrays, sections = {}, [HEADER]

    def emit(name, values, columns, codes):
        arrays[name] = values
        kind = "u32" if codes else "u8"
        lines = ["#[rustfmt::skip]",
                 f"pub(crate) const {name}: [{kind}; {len(values)}] = ["]
        for start in range(0, len(values), columns):
            row = values[start:start + columns]
            cells = [f"0x{v:05x}" if codes else f"{v:2d}" for v in row]
            lines.append("    " + ", ".join(cells) + ",")
        sections.append("\n".join(lines + ["];", ""]))

    for number, width in WIDTHS.items():
        pairs = pair_table(tabs, offsets[number], width)
        emit(f"CODES_{number}", [p[0] for p in pairs], min(width, 8), True)
        emit(f"LENS_{number}", [p[1] for p in pairs], width, False)
    quads = quad_table(c_array(source, "tab32"))
    emit("QUAD_A_CODES", [q[0] for q in quads], 8, True)
    emit("QUAD_A_LENS", [q[1] for q in quads], 16, False)
    return arrays, "\n".join(sections)


def compare_values(committed, generated):
    arrays = {}
    pattern = r"const (\w+): \[u(?:8|32); (\d+)\] = \[(.*?)\];"
    for name, size, body in re.findall(pattern, committed, re.S):
        values = [int(n, 0) for n in re.findall(r"0x[0-9a-f]+|\d+", body)]
        if name in arrays or len(values) != int(size):
            raise ValueError(f"{name}: duplicate array or incorrect declared length")
        arrays[name] = values
    if arrays.keys() != generated.keys():
        raise ValueError("committed and generated array names differ")
    for name, expected in generated.items():
        actual = arrays[name]
        if len(actual) != len(expected):
            raise ValueError(f"{name}: committed/generated lengths differ")
        for index, (old, new) in enumerate(zip(actual, expected)):
            if old != new:
                raise ValueError(
                    f"{name}[{index}]: committed={old}, minimp3={new}; refusing to write"
                )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--write", action="store_true")
    parser.add_argument("--source", type=Path, help="offline pinned minimp3.h")
    args = parser.parse_args()
    if args.source:
        data = args.source.read_bytes()
    else:
        with urllib.request.urlopen(URL, timeout=30) as response:
            data = response.read()
    if hashlib.sha256(data).hexdigest() != SHA256:
        raise ValueError("minimp3.h SHA-256 differs from the pinned source")
    arrays, rendered = generate(data.decode("utf-8"))
    if TARGET.exists():
        compare_values(TARGET.read_text(encoding="utf-8"), arrays)
    if args.write:
        TARGET.write_text(rendered, encoding="utf-8")
    elif TARGET.read_bytes() != rendered.encode("utf-8"):
        raise ValueError("values match, but generated file differs; run with --write")
    print(f"{'Regenerated' if args.write else 'Verified'} {len(arrays)} arrays "
          f"({sum(map(len, arrays.values()))} values) from minimp3 {COMMIT}")


if __name__ == "__main__":
    main()
