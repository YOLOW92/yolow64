// Standalone validation-build check for Turbo-Base64.
//
// Build the upstream library twice, then link this file against each build:
//
//   cc -O3 -mavx2 -I/tmp/Turbo-default turbo_check.c \
//      /tmp/Turbo-default/libtb64.a -o /tmp/turbo_default
//   cc -O3 -mavx2 -I/tmp/Turbo-full turbo_check.c \
//      /tmp/Turbo-full/libtb64.a -o /tmp/turbo_full
//
// Run a size in direct-kernel mode and in dispatch mode:
//
//   taskset -c 4 /tmp/turbo_full 65536 4096 0
//   taskset -c 4 /tmp/turbo_full 65536 4096 1
//
// The program prints the dispatch target once, then one CSV row:
//   mode,size,encoded_len,median_gbps
// Each GB/s value is the median of seven internal repetitions.

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "turbob64.h"

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec * 1e-9;
}

static int cmp_double(const void *a, const void *b) {
    double x = *(const double *)a;
    double y = *(const double *)b;
    return x < y ? -1 : x > y;
}

int main(int argc, char **argv) {
    size_t n = argc > 1 ? strtoull(argv[1], 0, 0) : 65536;
    int iters = argc > 2 ? atoi(argv[2]) : 4096;
    int use_dispatch = argc > 3 ? atoi(argv[3]) : 0;

    unsigned char *in = malloc(n);
    unsigned char *enc = malloc(tb64enclen(n) + 64);
    unsigned char *out = malloc(n + 64);
    if (!in || !enc || !out) {
        return 1;
    }
    for (size_t i = 0; i < n; i++) {
        in[i] = (unsigned char)(i * 73 + 19);
    }
    size_t enc_len = tb64enc(in, n, enc);
    if (!enc_len) {
        fprintf(stderr, "encode failed\n");
        return 1;
    }

    tb64ini(0, 0);
    fprintf(stderr, "_tb64d=%p tb64v256dec=%p _tb64v256dec=%p equal=%d\n",
            (void *)_tb64d, (void *)tb64v256dec, (void *)_tb64v256dec,
            _tb64d == tb64v256dec);

    double samples[7];
    for (int round = 0; round < 7; round++) {
        double start = now();
        for (int i = 0; i < iters; i++) {
            size_t written =
                use_dispatch ? tb64dec(enc, enc_len, out)
                             : tb64v256dec(enc, enc_len, out);
            if (written != n) {
                fprintf(stderr, "bad decoded length %zu (want %zu)\n", written, n);
                return 2;
            }
        }
        samples[round] = n * (double)iters / (now() - start) / 1e9;
    }
    qsort(samples, 7, sizeof(double), cmp_double);

    printf("%s,%zu,%zu,%.4f\n", use_dispatch ? "dispatch" : "direct", n,
           enc_len, samples[3]);
    return 0;
}
