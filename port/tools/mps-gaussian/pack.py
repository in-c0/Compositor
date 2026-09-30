"""Packs `trace_passes mps 20 5000` output into engine/src/adjust/mps_gaussian.bin.

    python pack.py table.txt ../../crates/engine/src/adjust/mps_gaussian.bin

Layout (little endian): b"MPSG", u32 version 2, first k, count of k, number of distinct sets; a
u16 set index per k; a u32 byte offset per set; then each set: u8 number of passes, and per pass
u8 kind (0 shrink, 1 blur, 2 grow), u8 log2 of its factor, u8 number of weights, f32 weights.
Only the vertical passes are kept: each horizontal twin carries the same weights.
"""
import struct
import sys

import numpy as np

KINDS = {'D': 0, 'F': 1, 'U': 2}


def parse(path):
    table = {}
    for line in open(path):
        parts = [p.strip() for p in line.split('|')]
        k = int(parts[0].split()[0])
        passes, i = [], 1
        while i < len(parts):
            label = parts[i]
            # The pipeline's label, then its constants (starting with the label again).
            if i + 1 < len(parts) and parts[i + 1].startswith(label):
                values = [np.float32(float(v)) for v in parts[i + 1].split()[1:]]
                i += 2
            else:
                values = []
                i += 1
            if not label.endswith('V'):
                continue
            if label.startswith('D'):
                n = int(label[label.index('F') + 1:-1]) // 2
                passes.append(('D', int(label[1:label.index('F')]), values[:n]))
            elif label.startswith('F'):
                passes.append(('F', 1, values[:int(label[1:-1]) // 2 + 1]))
            elif label.startswith('U'):
                passes.append(('U', int(label[1:label.index('P')]), values[:1]))
            else:
                raise ValueError(f'unknown pass {label} at k = {k}')
        table[k] = passes
    return table


def main(src, dst):
    table = parse(src)
    ks = sorted(table)
    assert ks == list(range(ks[0], ks[0] + len(ks))), 'k must be contiguous'
    index, blobs, ids = {}, [], []
    for k in ks:
        blob = bytearray(struct.pack('<B', len(table[k])))
        for kind, factor, weights in table[k]:
            blob += struct.pack('<BBB', KINDS[kind], factor.bit_length() - 1, len(weights))
            blob += b''.join(w.tobytes() for w in weights)
        blob = bytes(blob)
        if blob not in index:
            index[blob] = len(blobs)
            blobs.append(blob)
        ids.append(index[blob])
    offsets, at = [], 0
    for blob in blobs:
        offsets.append(at)
        at += len(blob)
    out = b'MPSG' + struct.pack('<IIII', 2, ks[0], len(ks), len(blobs))
    out += b''.join(struct.pack('<H', i) for i in ids)
    out += b''.join(struct.pack('<I', o) for o in offsets) + b''.join(blobs)
    with open(dst, 'wb') as f:
        f.write(out)
    print(f'{len(ks)} sigmas, {len(blobs)} distinct sets, {len(out)} bytes')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
