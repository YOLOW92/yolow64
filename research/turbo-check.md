# Turbo-Base64 validation-build verification

Measurement date: 2026-11-11. Upstream source:
`d9e584363280055ba6355938a48dc3711183f50f` (2026-06-28).

This note corrects an earlier description. Turbo-Base64's default AVX2 build
does **not** check every other block. In the 256-byte DS128 hot loop it checks
**one block out of four (25%)**. `FULLCHECK=1` selects the fully checking build.
The parity comparison in [`decode.md`](decode.md) uses that build.

## Macro mapping

`makefile` selects the define:

```make
ifeq ($(NCHECK),1)
DEFS+=-DNB64CHECK
else
ifeq ($(FULLCHECK),1)
DEFS+=-DB64CHECK
endif
endif
```

`turbob64_.h` consumes it:

```c
#ifdef NB64CHECK
#define CHECK0(a)
#define CHECK1(a)
#else
#define CHECK0(a) a
#ifdef B64CHECK
#define CHECK1(a) a
#else
#define CHECK1(a)
#endif
#endif
```

`DS128` in `turbob64v256.c` handles four 32-byte blocks. The first check is
`CHECK0`; the next three are `CHECK1`:

```c
CHECK0(B64CHK256(iu0, shiftedu0, check_asso, check_values, vx));
...
CHECK1(B64CHK256(iu1, shiftedu1, check_asso, check_values, vx));
...
CHECK1(B64CHK256(iv0, shiftedv0, check_asso, check_values, vx));
CHECK1(B64CHK256(iv1, shiftedv1, check_asso, check_values, vx));
```

Therefore:

- default build: `CHECK0` runs once per DS128 group, `CHECK1` is empty, so
  one of four blocks is checked in the hot loop;
- `FULLCHECK=1`: all four blocks are checked;
- the 16-byte tail loop uses `CHECK0` for every block in both builds.

The short-input entry point `_tb64v256dec` is a different kernel. It is not the
default dispatch target for the sizes measured here.

## Build verification

Two clean trees from the same source were built. The compile lines recorded by
`make` are:

```text
# default
cc -O3 -DNDEBUG -falign-loops -fPIC -fstrict-aliasing -march=haswell \
   turbob64v256.c -c -o turbob64v256.o

# full check
cc -O3 -DNDEBUG -falign-loops -DB64CHECK -fPIC -fstrict-aliasing -march=haswell \
   turbob64v256.c -c -o turbob64v256.o
```

`-DB64CHECK` occurs six times in the full build log (all translation units)
and zero times in the default log. Object sizes: default `libtb64.a` 67556 B,
full-check 68956 B.

## Dispatch target verification

`tb64dec` calls `_tb64d`, which `tb64ini(0,0)` sets to `tb64v256dec`. The
check program prints the pointers after initialization:

```text
_tb64d=0x...540 tb64v256dec=0x...540 _tb64v256dec=0x...540 equal=1
```

Only the addresses vary between runs; `_tb64d == tb64v256dec` and the short
kernel is at a different address. The benchmark therefore calls the same
kernel directly (`tb64v256dec`) and through `tb64dec`.

## Disassembly check

`objdump -d --disassemble=tb64v256dec` on the two objects:

| Build | `vpavgb` | `vpshufb` |
|---|---:|---:|
| default | 17 | 48 |
| full check | 26 | 65 |

The counts decompose exactly as expected:

- the hot region contains three `DS128` expansions (the two unrolled groups in
  the main loop and one 128-byte remainder);
- each `DS128` translates four blocks (`BITMAP256V8_6x` twice = 4 `vpavgb`)
  and checks four blocks in the full build (`B64CHK256` = 4 `vpavgb`);
- default checks 1 block per `DS128` => 3 checks; full checks 4 => 12 checks;
- the 16-byte tail adds one translate and one check (`vpavgb` each) in both
  builds.

Totals: default 12 translate + 3 checks + 2 tail = 17; full 12 + 12 + 2 = 26.
The difference of nine `vpavgb` is precisely the three extra checks per
`DS128` group, confirming 25% versus 100% in the hot loop.

## Benchmark

`research/turbo_check.c` is the standalone driver. Build the upstream library
twice, then link:

```sh
cc -O3 -mavx2 -I/tmp/Turbo-default research/turbo_check.c \
   /tmp/Turbo-default/libtb64.a -o /tmp/turbo_default
cc -O3 -mavx2 -I/tmp/Turbo-full research/turbo_check.c \
   /tmp/Turbo-full/libtb64.a -o /tmp/turbo_full
```

Sizes and iteration counts:

```sh
for n in 8192 16384 65536 262144 1048576 4194304; do
  case $n in
    8192) it=32768;; 16384) it=16384;; 65536) it=4096;;
    262144) it=1024;; 1048576) it=256;; 4194304) it=64;;
  esac
  for mode in 0 1; do
    taskset -c 4 /tmp/turbo_default "$n" "$it" "$mode"
    taskset -c 4 /tmp/turbo_full    "$n" "$it" "$mode"
  done
done
```

Mode `0` calls `tb64v256dec` directly; mode `1` calls `tb64dec` after
`tb64ini(0,0)`. Both modes use the same kernel; the small differences below are
code layout and run-to-run variation. The full CSV is
[`../benchmarks/results/turbo_check_verify.csv`](../benchmarks/results/turbo_check_verify.csv).

GB/s, median of seven internal repetitions:

| Input | default direct | default dispatch | full direct | full dispatch |
|---:|---:|---:|---:|---:|
| 8 KiB | 26.34 | 26.89 | 20.06 | 19.49 |
| 16 KiB | 26.41 | 26.70 | 19.68 | 19.62 |
| 64 KiB | 26.90 | 26.76 | 19.36 | 19.38 |
| 256 KiB | 20.80 | 20.72 | 16.76 | 16.84 |
| 1 MiB | 20.57 | 20.67 | 16.68 | 16.76 |
| 4 MiB | 17.01 | 17.18 | 13.89 | 14.18 |

The default/full gap (about 1.35x at 8–64 KiB) matches the 4:1 check ratio.
The earlier 26/20 GB/s pairing was therefore a partial-check versus full-check
comparison, not a full-check versus full-check comparison. The earlier phrase
"alternating blocks" in this repository meant "only the `CHECK0` subset"; it
was inaccurate and has been replaced with the 25% description above.
