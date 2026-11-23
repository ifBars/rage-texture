/* Offline validation only. Build with externally supplied bcdec.h from
 * iOrange/bcdec at 80859ed3b7afb1c527a2a99d70c61457bea72d0c.
 * Its implementation is not bundled or linked into the product.
 * Usage: oracle SYNTHETIC_BLOCKS OUTPUT SIGNED(0|1). */
#define BCDEC_IMPLEMENTATION
#include "bcdec.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(int argc, char **argv) {
    if (argc != 4 || (strcmp(argv[3], "0") && strcmp(argv[3], "1"))) return 2;
    FILE *in = fopen(argv[1], "rb"), *out = fopen(argv[2], "wb");
    if (!in || !out) return 3;
    unsigned char block[16];
    unsigned short pixels[48];
    size_t n;
    while ((n = fread(block, 1, 16, in)) == 16) {
        bcdec_bc6h_half(block, pixels, 12, atoi(argv[3]));
        for (size_t i = 0; i < 48; ++i) {
            unsigned char bytes[2] = {(unsigned char)pixels[i], (unsigned char)(pixels[i] >> 8)};
            if (fwrite(bytes, 1, 2, out) != 2) return 4;
        }
    }
    int failed = n != 0 || ferror(in) || fclose(out) != 0;
    fclose(in);
    return failed ? 5 : 0;
}
