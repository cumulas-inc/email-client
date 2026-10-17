#!/usr/bin/env python3
"""Generates crates/core/src/providers/graph/windows_zones.rs: the Windows
time zone for each IANA time zone name, which Microsoft Graph's
dateTimeTimeZone accepts everywhere.

The mapping is Unicode CLDR's (common/supplemental/windowsZones.xml). CLDR
names zones the old way (Asia/Calcutta, Europe/Kiev), so tzdata's
`backward` links fill in the rest: every link name of a mapped zone gets
that zone's Windows name, and a zone CLDR knows only by one of its link
names gets that link's. CLDR's own entries always win.

Download the two files at the pinned versions below, then run:

    curl -fLO https://raw.githubusercontent.com/unicode-org/cldr/release-48-2/common/supplemental/windowsZones.xml
    curl -fLO https://raw.githubusercontent.com/eggert/tz/2026e/backward
    python3 scripts/windows-zones.py windowsZones.xml backward \\
        > crates/core/src/providers/graph/windows_zones.rs

The same files always give the same output, byte for byte. To update,
change the versions here, download them again and rerun.
"""

import sys
import xml.etree.ElementTree as ET

CLDR_RELEASE = "release-48-2"
TZDATA_VERSION = "2026e"

# The notice CLDR's data is distributed under (LICENSE at CLDR_RELEASE),
# which must accompany copies of it.
UNICODE_LICENSE = """\
UNICODE LICENSE V3

COPYRIGHT AND PERMISSION NOTICE

Copyright © 2004-2026 Unicode, Inc.

NOTICE TO USER: Carefully read the following legal agreement. BY
DOWNLOADING, INSTALLING, COPYING OR OTHERWISE USING DATA FILES, AND/OR
SOFTWARE, YOU UNEQUIVOCALLY ACCEPT, AND AGREE TO BE BOUND BY, ALL OF THE
TERMS AND CONDITIONS OF THIS AGREEMENT. IF YOU DO NOT AGREE, DO NOT
DOWNLOAD, INSTALL, COPY, DISTRIBUTE OR USE THE DATA FILES OR SOFTWARE.

Permission is hereby granted, free of charge, to any person obtaining a
copy of data files and any associated documentation (the "Data Files") or
software and any associated documentation (the "Software") to deal in the
Data Files or Software without restriction, including without limitation
the rights to use, copy, modify, merge, publish, distribute, and/or sell
copies of the Data Files or Software, and to permit persons to whom the
Data Files or Software are furnished to do so, provided that either (a)
this copyright and permission notice appear with all copies of the Data
Files or Software, or (b) this copyright and permission notice appear in
associated Documentation.

THE DATA FILES AND SOFTWARE ARE PROVIDED "AS IS", WITHOUT WARRANTY OF ANY
KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT OF
THIRD PARTY RIGHTS.

IN NO EVENT SHALL THE COPYRIGHT HOLDER OR HOLDERS INCLUDED IN THIS NOTICE
BE LIABLE FOR ANY CLAIM, OR ANY SPECIAL INDIRECT OR CONSEQUENTIAL DAMAGES,
OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS,
WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION,
ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THE DATA
FILES OR SOFTWARE.

Except as contained in this notice, the name of a copyright holder shall
not be used in advertising or otherwise to promote the sale, use or other
dealings in these Data Files or Software without prior written
authorization of the copyright holder.

SPDX-License-Identifier: Unicode-3.0
"""


def fail(message):
    sys.exit(f"windows-zones.py: {message}")


def plain(name, what):
    """`name`, checked to be printable ASCII without quotes or backslashes, so
    it sorts the same in Rust and needs no escaping there."""
    if not name or not all(" " <= c <= "~" and c not in '"\\' for c in name):
        fail(f"unexpected {what}: {name!r}")
    return name


def cldr_zones(path):
    """IANA name -> Windows name, from CLDR's windowsZones.xml."""
    zones = {}
    for entry in ET.parse(path).getroot().iter("mapZone"):
        windows = plain(entry.get("other", ""), "Windows zone")
        for iana in entry.get("type", "").split():
            plain(iana, "IANA zone")
            if zones.setdefault(iana, windows) != windows:
                fail(f"CLDR maps {iana} to both {zones[iana]} and {windows}")
    if not zones:
        fail(f"no mapZone entries in {path}")
    return zones


def backward_links(path):
    """(target, link name) pairs from tzdata's backward file."""
    links = []
    with open(path, encoding="utf-8") as lines:
        for line in lines:
            fields = line.split("#", 1)[0].split()
            if fields and fields[0] == "Link":
                if len(fields) != 3:
                    fail(f"unexpected line in {path}: {line.strip()}")
                links.append(
                    (plain(fields[1], "link target"), plain(fields[2], "link name"))
                )
    if not links:
        fail(f"no Link lines in {path}")
    return links


def windows_zones(zones, links):
    mapped = dict(zones)
    names = {}
    for target, name in links:
        names.setdefault(target, []).append(name)
    # A zone CLDR knows only by a link name: the link's Windows zone (the
    # alphabetically first link's, should they ever differ).
    for target in sorted(names):
        if target in mapped:
            continue
        known = sorted(n for n in names[target] if n in zones)
        if known:
            if len({zones[n] for n in known}) > 1:
                print(
                    f"note: the links of {target} differ; using {known[0]}'s",
                    file=sys.stderr,
                )
            mapped[target] = zones[known[0]]
    # Every link name of a mapped zone that CLDR doesn't name itself.
    for target in sorted(names):
        if target in mapped:
            for name in names[target]:
                mapped.setdefault(name, mapped[target])
    return mapped


def render(mapped):
    out = [
        "//! The Windows time zone for each IANA time zone name, which Microsoft",
        "//! Graph's `dateTimeTimeZone` accepts everywhere.",
        "//!",
        "//! Generated by `scripts/windows-zones.py`; don't edit it by hand. Sources:",
        f"//! Unicode CLDR {CLDR_RELEASE}, `common/supplemental/windowsZones.xml`, and",
        f"//! the `backward` file of tzdata {TZDATA_VERSION} (public domain), whose links add",
        "//! the names CLDR doesn't use.",
        "//!",
        "//! The CLDR data is used under this notice:",
        "//!",
    ]
    for line in UNICODE_LICENSE.splitlines():
        out.append(f"//! {line}".rstrip())
    out += [
        "",
        "/// Windows time zone names by IANA name, sorted by IANA name.",
        "#[rustfmt::skip]",
        "static WINDOWS_ZONES: &[(&str, &str)] = &[",
    ]
    for iana in sorted(mapped):
        out.append(f'    ("{iana}", "{mapped[iana]}"),')
    out += [
        "];",
        "",
        "/// The Windows time zone for an IANA time zone name; `None` when neither",
        "/// CLDR nor tzdata's links know it.",
        "pub(super) fn windows_zone(iana: &str) -> Option<&'static str> {",
        "    WINDOWS_ZONES",
        "        .binary_search_by(|(name, _)| (*name).cmp(iana))",
        "        .ok()",
        "        .map(|at| WINDOWS_ZONES[at].1)",
        "}",
        "",
    ]
    return "\n".join(out)


def main():
    if len(sys.argv) != 3:
        fail("usage: windows-zones.py WINDOWS_ZONES_XML BACKWARD > windows_zones.rs")
    mapped = windows_zones(cldr_zones(sys.argv[1]), backward_links(sys.argv[2]))
    sys.stdout.buffer.write(render(mapped).encode("utf-8"))


if __name__ == "__main__":
    main()
