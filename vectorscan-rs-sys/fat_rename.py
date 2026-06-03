#!/usr/bin/env python3
# Windows/COFF fat-runtime symbol renamer (whole-variant pass).
#
# Usage: fat_rename.py <prefix> <keepsyms_file> <object_dir>
#
# build.rs copies this into the vectorscan source tree's cmake/ dir for MSVC
# fat-runtime builds; the patched CMakeLists.txt (patch 08) runs it once per
# micro-arch variant.
#
# The upstream POSIX build_wrapper.sh renames symbols one object at a time,
# relying on a dump of libc's dynamic symbols to know what NOT to rename. That
# doesn't port to the MSVC toolchain (no libc.so to enumerate, no shell), and a
# per-object pass cannot rename a *reference* to a symbol defined in a sibling
# object. Instead we run once per variant after all its objects exist:
#   1. collect every external *defined* symbol across the variant's objects
#      -> these are exactly the symbols the variant owns;
#   2. apply that single rename map ("sym" -> "<prefix>_sym") to *every* object
#      with llvm-objcopy --redefine-syms, which rewrites both the definitions
#      and any (undefined) references to them.
# Symbols the variant does not define -- the CRT (memcpy, ...) and the shared
# allocator hooks in hs_exec_common -- are never in the map, so references to
# them are left intact and still resolve at link time.
import glob
import os
import re
import subprocess
import sys

DEFINED_TYPES = set("TDBR")  # external, defined sections (uppercase == external)


def load_keep(path):
    names, patterns = set(), []
    with open(path) as fh:
        for line in fh:
            tok = line.strip()
            if not tok or tok.startswith("#"):
                continue
            if re.search(r"[\^$*\[\].]", tok):
                patterns.append(re.compile(tok))
            else:
                names.add(tok)
    return names, patterns


def nm_symbols(obj):
    out = subprocess.run(["llvm-nm", obj], capture_output=True, text=True)
    if out.returncode != 0:
        sys.stderr.write(out.stderr)
        raise SystemExit(out.returncode)
    for line in out.stdout.splitlines():
        parts = line.split()
        if len(parts) >= 3:
            yield parts[1], parts[2]
        elif len(parts) == 2:
            yield parts[0], parts[1]


def main():
    prefix, keepfile, objdir = sys.argv[1], sys.argv[2], sys.argv[3]
    keep_names, keep_patterns = load_keep(keepfile)
    objs = glob.glob(os.path.join(objdir, "**", "*.obj"), recursive=True)
    if not objs:
        return 0

    owned = set()
    for obj in objs:
        for sym_type, name in nm_symbols(obj):
            if sym_type not in DEFINED_TYPES:
                continue
            if name.startswith(prefix + "_"):
                continue  # already renamed (idempotent re-runs)
            if name in keep_names or any(p.search(name) for p in keep_patterns):
                continue
            owned.add(name)
    if not owned:
        return 0

    mapfile = os.path.join(objdir, "fat_rename_%s.syms" % prefix)
    with open(mapfile, "w") as fh:
        for name in sorted(owned):
            fh.write("%s %s_%s\n" % (name, prefix, name))
    try:
        for obj in objs:
            oc = subprocess.run(
                ["llvm-objcopy", "--redefine-syms=" + mapfile, obj],
                capture_output=True, text=True)
            if oc.returncode != 0:
                sys.stderr.write(oc.stderr)
                return oc.returncode
    finally:
        os.remove(mapfile)
    return 0


if __name__ == "__main__":
    sys.exit(main())
