#include <stdio.h>
#include <stdlib.h>
int main(void) {
    long n = 20000000, count = 0;
    char *flags = malloc(n + 1);
    for (long i = 0; i <= n; i++) flags[i] = 1;
    for (long p = 2; p <= n; p++) {
        if (flags[p]) {
            count++;
            for (long m = p * p; m <= n; m += p) flags[m] = 0;
        }
    }
    printf("%ld\n", count);
    return 0;
}
