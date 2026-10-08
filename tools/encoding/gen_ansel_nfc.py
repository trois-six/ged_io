#!/usr/bin/env python3
"""Generates src/encoding/ansel_nfc.rs: the ANSEL canonical composition tables.

ANSEL (ANSI/NISO Z39.47, GEDCOM 5.5.1 Appendix C) writes a letter with a
diacritic as one or more non-spacing marks followed by the base character.
Unicode NFC writes the base first and composes it with its marks wherever a
precomposed character exists. This script derives, from Python's
`unicodedata` (the Unicode Character Database), every composition reachable
from an ANSEL base character with ANSEL's combining marks:

* `CCC`: the canonical combining class of each ANSEL mark;
* `COMPOSE`: (starter, mark) -> primary composite, closed under composition
  (a composite is itself a starter for the next mark, as in a + U+0302 ->
  U+00E2, then U+00E2 + U+0301 -> U+1EA5);
* `DECOMPOSE`: the inverse, used to write precomposed text back to ANSEL.

The algorithm the decoder applies (canonical ordering of the marks, then
canonical composition with blocking) is re-implemented below and checked
against `unicodedata.normalize("NFC", ...)` for every base with every single
mark, every ordered pair of marks, and every ordered triple of the most
common marks. Generation fails if any result differs.

Standard library only. Usage: python3 tools/encoding/gen_ansel_nfc.py > src/encoding/ansel_nfc.rs && rustfmt src/encoding/ansel_nfc.rs
"""

import itertools
import sys
import unicodedata

# ANSEL non-spacing marks (byte -> Unicode combining character), as decoded by
# src/encoding/tables.rs (`ansel_mark`); the `ansel_marks_have_a_class` unit
# test keeps the two in sync.
MARKS = {
    0xE0: 0x0309, 0xE1: 0x0300, 0xE2: 0x0301, 0xE3: 0x0302, 0xE4: 0x0303,
    0xE5: 0x0304, 0xE6: 0x0306, 0xE7: 0x0307, 0xE8: 0x0308, 0xE9: 0x030C,
    0xEA: 0x030A, 0xEB: 0xFE20, 0xEC: 0xFE21, 0xED: 0x0315, 0xEE: 0x030B,
    0xEF: 0x0310, 0xF0: 0x0327, 0xF1: 0x0328, 0xF2: 0x0323, 0xF3: 0x0324,
    0xF4: 0x0325, 0xF5: 0x0333, 0xF6: 0x0332, 0xF7: 0x0326, 0xF8: 0x031C,
    0xF9: 0x032E, 0xFA: 0xFE22, 0xFB: 0xFE23, 0xFE: 0x0313,
}

# ANSEL spacing characters above 0x7F that can carry a mark.
SPECIALS = [
    0x0141, 0x00D8, 0x0110, 0x00DE, 0x00C6, 0x0152, 0x02B9, 0x00B7, 0x266D,
    0x00AE, 0x00B1, 0x01A0, 0x01AF, 0x02BC, 0x02BB, 0x0142, 0x00F8, 0x0111,
    0x00FE, 0x00E6, 0x0153, 0x02BA, 0x0131, 0x00A3, 0x00F0, 0x01A1, 0x01B0,
    0x00B0, 0x2113, 0x2117, 0x00A9, 0x266F, 0x00BF, 0x00A1, 0x00DF, 0x20AC,
    0x25A1, 0x25A0,
]

ansel_marks = [chr(m) for m in MARKS.values()]
bases = [chr(c) for c in range(0x21, 0x7F)] + [chr(c) for c in SPECIALS]

# Some ANSEL spacing characters are themselves composites (U+01A0 is O with
# a horn): their marks take part in the composition too.
marks = list(ansel_marks)
starters = []
for b in bases:
    nfd = unicodedata.normalize("NFD", b)
    if nfd[0] not in starters:
        starters.append(nfd[0])
    for m in nfd[1:]:
        if m not in marks:
            marks.append(m)
ccc = {m: unicodedata.combining(m) for m in marks}
assert all(c > 0 for c in ccc.values()), "every mark must be non-spacing"

compose = {}
frontier = list(starters)
seen = set(frontier)
while frontier:
    nxt = []
    for s in frontier:
        for m in marks:
            c = unicodedata.normalize("NFC", s + m)
            # Keep primary compositions only: NFC may also reorder marks into
            # an existing composite (U+00C2 + U+0323 gives U+1EAC, whose
            # canonical decomposition is U+1EA0 + U+0302); the decoder
            # decomposes and sorts the marks first, so it never needs those.
            if len(c) == 1 and unicodedata.decomposition(c).split() == [
                "%04X" % ord(s),
                "%04X" % ord(m),
            ]:
                compose[(s, m)] = c
                if c not in seen:
                    seen.add(c)
                    nxt.append(c)
    frontier = nxt

decompose = {}
for (s, m), c in compose.items():
    assert c not in decompose
    decompose[c] = (s, m)
for b in bases:
    assert unicodedata.normalize("NFD", b) == b or b in decompose, b


def nfc(base, ms):
    """The decoder's algorithm: decompose the base, sort the marks by class
    (stable), then compose each mark that is not blocked."""
    head = []
    while base in decompose:
        base, m = decompose[base]
        head.insert(0, m)
    ms = sorted(head + list(ms), key=lambda m: ccc[m])
    starter, rest, last = base, [], None
    for m in ms:
        blocked = last is not None and last >= ccc[m]
        if not blocked and (starter, m) in compose:
            starter = compose[(starter, m)]
            continue
        rest.append(m)
        last = ccc[m]
    return starter + "".join(rest)


checked = 0
for b in bases:
    for n in (1, 2):
        for ms in itertools.product(ansel_marks, repeat=n):
            want = unicodedata.normalize("NFC", b + "".join(ms))
            got = nfc(b, ms)
            assert got == want, (b, ms, got, want)
            checked += 1
common = [chr(c) for c in (0x0300, 0x0301, 0x0302, 0x0303, 0x0308, 0x0323, 0x0327, 0x0306)]
for b in bases:
    for ms in itertools.product(common, repeat=3):
        want = unicodedata.normalize("NFC", b + "".join(ms))
        assert nfc(b, ms) == want, (b, ms)
        checked += 1


def u(c):
    return "'\\u{%04X}'" % ord(c)


out = sys.stdout
out.write("// @generated by tools/encoding/gen_ansel_nfc.py from the Unicode Character Database\n")
out.write("// %s (Python unicodedata). Do not edit by hand: re-run the script.\n" % unicodedata.unidata_version)
out.write("// %d compositions; %d NFC results cross-checked at generation time.\n\n" % (len(compose), checked))
out.write("/// Canonical combining class of an ANSEL combining mark (0 for any other character).\n")
out.write("pub(super) const fn ccc(mark: char) -> u8 {\n    match mark {\n")
by_class = {}
for m in sorted(marks):
    by_class.setdefault(ccc[m], []).append(m)
for k in sorted(by_class):
    out.write("        %s => %d,\n" % (" | ".join(u(m) for m in by_class[k]), k))
out.write("        _ => 0,\n    }\n}\n\n")
out.write("/// `(starter, mark, composite)`, sorted by `(starter, mark)`.\n")
out.write("pub(super) static COMPOSE: [(char, char, char); %d] = [\n" % len(compose))
for (s, m), c in sorted(compose.items(), key=lambda kv: (ord(kv[0][0]), ord(kv[0][1]))):
    out.write("    (%s, %s, %s),\n" % (u(s), u(m), u(c)))
out.write("];\n\n")
out.write("/// `(composite, starter, mark)`, sorted by composite.\n")
out.write("pub(super) static DECOMPOSE: [(char, char, char); %d] = [\n" % len(decompose))
for c, (s, m) in sorted(decompose.items(), key=lambda kv: ord(kv[0])):
    out.write("    (%s, %s, %s),\n" % (u(c), u(s), u(m)))
out.write("];\n")
