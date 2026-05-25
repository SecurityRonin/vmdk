#!/usr/bin/env bash
# Generate VMDK corpus images using qemu-img.
# Requires: qemu-utils (sudo apt-get install -y qemu-utils)
set -euo pipefail

DEST="$(cd "$(dirname "$0")" && pwd)"

# Monolithic sparse — the format we support
qemu-img create -f vmdk "${DEST}/sparse.vmdk" 10M

# Write a recognisable data pattern so sector reads are non-trivial
python3 - "${DEST}/sparse.vmdk" <<'PY'
import struct, sys
pattern = bytes(range(256)) * 2   # 512 bytes recognisable pattern
with open(sys.argv[1], "r+b") as f:
    f.seek(0, 2); size = f.tell()
    # Write at offset 65536 (first grain boundary in default VMDK)
    if size > 65536 + 512:
        f.seek(65536); f.write(pattern)
PY
