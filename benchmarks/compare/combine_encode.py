#!/usr/bin/env python3
"""Combine three encode_all summary CSV files into one median table."""
import csv
import statistics
import sys
from collections import defaultdict

if len(sys.argv) < 4:
    raise SystemExit("usage: combine_encode.py RUN1.csv RUN2.csv RUN3.csv > combined.csv")

values = defaultdict(list)
for path in sys.argv[1:]:
    with open(path, newline="") as f:
        for row in csv.DictReader(f):
            key = (row["mode"], int(row["size"]), int(row["align"]), row["lib"], row["kind"])
            values[key].append(float(row["median_ns"]))

writer = csv.writer(sys.stdout)
writer.writerow(["mode", "size", "align", "lib", "kind", "median_ns", "gbps"])
for key in sorted(values, key=lambda k: (k[0], k[1], k[2], k[3])):
    mode, size, align, lib, kind = key
    ns = statistics.median(values[key])
    writer.writerow([mode, size, align, lib, kind, f"{ns:.6f}", f"{size / ns:.6f}"])
