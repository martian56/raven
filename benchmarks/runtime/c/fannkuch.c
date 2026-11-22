#include <stdio.h>
int main(void) {
    long n = 10, perm[16], perm1[16], count[16];
    for (long i = 0; i < n; i++) perm1[i] = i;
    long max_flips = 0, checksum = 0, perm_count = 0, r = n;
    for (;;) {
        while (r != 1) { count[r - 1] = r; r--; }
        for (long k = 0; k < n; k++) perm[k] = perm1[k];
        long flips = 0, first = perm[0];
        while (first != 0) {
            long lo = 0, hi = first;
            while (lo < hi) { long t = perm[lo]; perm[lo] = perm[hi]; perm[hi] = t; lo++; hi--; }
            flips++;
            first = perm[0];
        }
        if (flips > max_flips) max_flips = flips;
        checksum += perm_count % 2 == 0 ? flips : -flips;
        for (;;) {
            if (r == n) { printf("%ld\n%ld\n", checksum, max_flips); return 0; }
            long p0 = perm1[0];
            for (long j = 0; j < r; j++) perm1[j] = perm1[j + 1];
            perm1[r] = p0;
            count[r]--;
            if (count[r] > 0) break;
            r++;
        }
        perm_count++;
    }
}
