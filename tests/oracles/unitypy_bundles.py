"""Census of UnityFS bundles with an independent reader.

For every bundle given, prints JSON lines: the bundle's header, block counts and node count, then one line per node
with its offset, size, flags and SHA-256, and for a serialized node what UnityPy reads from it: the container version,
Unity version, target platform, every type (class id, stripped flag, script index and id, old type hash, a digest of the
stored type tree, dependencies), the object table (count and a digest of path id, class id, offset, size and type
index), script types, externals, reference types and the user string. The bundle layout is read by this script's own
reader (written from the format, not from unity-formats), so the node bytes are compared with a second implementation
even where UnityPy's whole-bundle loader would not fit memory (PEAK's level bundles are 2 GB each).

Usage:
    pip install UnityPy lz4
    python tests/oracles/unitypy_bundles.py <bundle>... > <oracles root>/unitypy/<game>/bundles.jsonl

`<oracles root>` is the `[oracles] root` of the GAME_DATA file (tests/inputs/README.md); tests/bundles.rs compares the
lines with what this crate reads. The digests are SHA-256 over the lines `level|type|name|byte size|version|meta flags`
of a tree's nodes in depth-first order, and `path id|class id|offset|size|type index` of the objects sorted by path id.
"""

import bisect
import hashlib
import io
import json
import lzma
import os
import struct
import sys

import lz4.block
from UnityPy.files import SerializedFile
from UnityPy.streams import EndianBinaryReader


def cstr(f):
    b = bytearray()
    while True:
        c = f.read(1)
        if c == b"\0" or not c:
            return b.decode()
        b += c


def decompress(data, compression, size):
    if compression == 0:
        return data
    if compression == 1:
        props = data[:5]
        return lzma.LZMADecompressor(format=lzma.FORMAT_ALONE).decompress(props + struct.pack("<Q", size) + data[5:])
    if compression in (2, 3):
        return lz4.block.decompress(data, uncompressed_size=size)
    raise ValueError(f"compression {compression}")


class Bundle:
    """The header, blocks and nodes of a UnityFS bundle (format 7 or 8), blocks read on demand."""

    def __init__(self, path):
        self.f = f = open(path, "rb")
        assert f.read(8) == b"UnityFS\0"
        (self.version,) = struct.unpack(">I", f.read(4))
        self.player, self.engine = cstr(f), cstr(f)
        self.size, csize, usize, self.flags = struct.unpack(">QIII", f.read(20))
        if self.version >= 7:
            f.seek((f.tell() + 15) & ~15)
        if self.flags & 0x80:
            here = f.tell()
            f.seek(self.size - csize)
            raw = f.read(csize)
            f.seek(here)
        else:
            raw = f.read(csize)
        info = decompress(raw, self.flags & 0x3F, usize)
        self.data_start = f.tell()
        if self.flags & 0x200:
            self.data_start = (self.data_start + 15) & ~15
        b = io.BytesIO(info)
        b.read(16)
        (n,) = struct.unpack(">i", b.read(4))
        self.blocks = [struct.unpack(">IIH", b.read(10)) for _ in range(n)]
        (m,) = struct.unpack(">i", b.read(4))
        self.nodes = []
        for _ in range(m):
            off, sz, fl = struct.unpack(">qqI", b.read(20))
            self.nodes.append((off, sz, fl, cstr(b)))
        assert b.tell() == len(info)
        self.ustart, self.fstart = [], []
        u, p = 0, self.data_start
        for usz, csz, _ in self.blocks:
            self.ustart.append(u)
            self.fstart.append(p)
            u += usz
            p += csz
        assert p == self.size, "the blocks end where the file does"

    def read(self, lo, hi):
        """Bytes [lo, hi) of the blocks' decompressed data."""
        out = bytearray()
        i = bisect.bisect_right(self.ustart, lo) - 1
        while i < len(self.blocks) and self.ustart[i] < hi:
            usz, csz, fl = self.blocks[i]
            self.f.seek(self.fstart[i])
            d = decompress(self.f.read(csz), fl & 0x3F, usz)
            assert len(d) == usz
            out += d[max(lo, self.ustart[i]) - self.ustart[i] : min(hi, self.ustart[i] + usz) - self.ustart[i]]
            i += 1
        return bytes(out)


def digest(lines):
    h = hashlib.sha256()
    for line in lines:
        h.update(line.encode())
        h.update(b"\n")
    return h.hexdigest()


def tree_lines(node):
    out = []

    def walk(n, level):
        out.append(f"{level}|{n.m_Type}|{n.m_Name}|{n.m_ByteSize}|{n.m_Version}|{n.m_MetaFlag}")
        for c in n.m_Children:
            walk(c, level + 1)

    walk(node, 0)
    return out


def serialized(data, name):
    f = SerializedFile(EndianBinaryReader(data), None, name)
    types = []
    for t in f.types:
        tree = None
        if t.node is not None:
            lines = tree_lines(t.node)
            tree = {"nodes": len(lines), "digest": digest(lines)}
        types.append(
            {
                "class_id": t.class_id,
                "stripped": bool(t.is_stripped_type),
                "script_type_index": t.script_type_index,
                "script_id": t.script_id.hex() if t.script_id is not None else None,
                "old_type_hash": t.old_type_hash.hex(),
                "tree": tree,
                "dependencies": list(t.type_dependencies or ()),
            }
        )
    objs = sorted(f.objects.values(), key=lambda o: o.path_id)
    index = {id(t): i for i, t in enumerate(f.types)}
    lines = [f"{o.path_id}|{o.class_id}|{o.byte_start}|{o.byte_size}|{index[id(o.serialized_type)]}" for o in objs]
    return {
        "version": f.header.version,
        "unity_version": f.unity_version,
        "target_platform": int(f.target_platform),
        "types": types,
        "objects": {"count": len(objs), "digest": digest(lines)},
        "script_types": [[s.local_serialized_file_index, s.local_identifier_in_file] for s in f.script_types],
        "externals": [e.path for e in f.externals],
        "ref_types": len(f.ref_types or ()),
        "user_information": f.userInformation or "",
    }


def main():
    for path in sys.argv[1:]:
        name = os.path.basename(path)
        b = Bundle(path)
        by_compression = {}
        for _, _, fl in b.blocks:
            by_compression[fl & 0x3F] = by_compression.get(fl & 0x3F, 0) + 1
        total = sum(x[0] for x in b.blocks)
        print(
            json.dumps(
                {
                    "bundle": name,
                    "header": {
                        "format_version": b.version,
                        "player_version": b.player,
                        "engine_version": b.engine,
                        "size": b.size,
                        "flags": b.flags,
                    },
                    "blocks": {str(k): v for k, v in sorted(by_compression.items())},
                    "data_len": total,
                    "block_bytes": total,
                    "nodes": len(b.nodes),
                }
            ),
            flush=True,
        )
        for off, sz, fl, nm in b.nodes:
            sha = hashlib.sha256()
            data = bytearray() if fl & 4 else None
            at = 0
            while at < sz:
                n = min(16 << 20, sz - at)
                piece = b.read(off + at, off + at + n)
                assert len(piece) == n
                sha.update(piece)
                if data is not None:
                    data += piece
                at += n
            rec = {"bundle": name, "node": nm, "offset": off, "size": sz, "flags": fl, "sha256": sha.hexdigest()}
            if fl & 4:
                try:
                    rec["serialized"] = serialized(bytes(data), nm)
                except Exception as e:  # a node UnityPy cannot read is recorded, not fatal
                    rec["serialized"] = {"error": f"{type(e).__name__}: {e}"}
            print(json.dumps(rec), flush=True)


main()
