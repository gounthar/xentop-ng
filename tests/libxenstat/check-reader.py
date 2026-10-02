#!/usr/bin/env python3
"""Compile the reader from both patches; exercise hostile files without Xen/root."""
from pathlib import Path
import json
import os
import re
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
FIELDS = 'oo_reqs rd_reqs wr_reqs rd_sects wr_sects rd_reqs_done wr_reqs_done rd_usecs wr_usecs io_errors'.split()
STATS = 'oo_reqs read_reqs_submitted read_reqs_completed read_sectors read_total_ticks write_reqs_submitted write_reqs_completed write_sectors write_total_ticks io_errors flags'.split()
HEAD = '''#define _GNU_SOURCE
#include <fcntl.h>
#include <errno.h>
#include <limits.h>
#include <sys/stat.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
'''
HEAD += 'typedef struct { unsigned int has_ext; ' + ''.join('uint64_t '+f+';' for f in FIELDS) + '} xenstat_vbd;\n'
HEAD += 'struct vbd3_stats { uint32_t version, pad; ' + ''.join('uint64_t '+f+';' for f in STATS) + '};\n'
MAIN = '''
int main(int argc, char **argv) {
    xenstat_vbd v = {0};
    if (argc != 2) return 2;
    int r = read_attributes_vbd3(argv[1], 5, 51712, &v);
    if (r == 0 && (v.rd_reqs != 1 || !v.has_ext)) return 3;
    if (r != 0 && v.has_ext) return 4;
    return r == 0 ? 0 : 1;
}
'''
for variant in ['xcp-ng-4.17', 'upstream']:
    patch = next((ROOT/'libxenstat'/variant).glob('0005-*')).read_text()
    # Reconstruct the new-side hunk text, retaining unchanged context.
    text = '\n'.join(l[1:] for l in patch.splitlines() if l.startswith((' ', '+')) and not l.startswith('+++'))
    reader = re.search(r'/\* Open each.*?\n}\n\nstatic int read_attributes_vbd\(', text, re.S).group().rsplit('\n\nstatic int ', 1)[0]
    with tempfile.TemporaryDirectory(prefix='xenstat-reader-') as tmp:
        root = Path(tmp)
        d = root/'td3-123'; d.mkdir(mode=0o755)
        f = d/'vbd-5-51712'
        good = struct.pack('=II11Q', 1, 0, *range(11))
        f.write_bytes(good); f.chmod(0o600)
        reader = reader.replace('"/dev/shm"', json.dumps(tmp))
        def compile_reader(body):
            (root/'test.c').write_text(HEAD+body+MAIN)
            subprocess.run(['cc', '-Wall', '-Wextra', '-Werror', '-o', str(root/'test'), str(root/'test.c')], check=True)
        def check(ok, pid='123'):
            p = subprocess.run([str(root/'test'), pid], timeout=2)
            assert p.returncode == (0 if ok else 1), (variant, pid, p.returncode)
        compile_reader(reader)
        check(os.geteuid() == 0)  # Exact production ownership policy.
        # Exercise all other guards as an unprivileged user, allowing only
        # this user's UID in the test build (no production override).
        compile_reader(reader.replace('st.st_uid != 0', 'st.st_uid != geteuid()'))
        check(True)
        for pid in ['', '0', '-1', '../123', '12/3', '2147483648', '9'*100]: check(False, pid)
        f.chmod(0o666); check(False); f.chmod(0o600)
        d.chmod(0o777); check(False); d.chmod(0o755)
        for data in [b'', good[:20], struct.pack('=I', 2)+good[4:], good+b'\0'*65536]:
            f.write_bytes(data); check(False)
        f.write_bytes(good)
        os.link(f, d/'hardlink'); check(False); (d/'hardlink').unlink()
        f.rename(d/'real'); f.symlink_to(d/'real'); check(False); f.unlink()
        os.mkfifo(f); check(False); f.unlink()
        f.mkdir(); check(False); f.rmdir()
        (d/'real').rename(f)
        d.rename(root/'real-dir'); d.symlink_to(root/'real-dir', target_is_directory=True); check(False)
    print(variant+': ownership, modes, symlinks, hardlinks, FIFO, sizes, version and PID checks passed')
