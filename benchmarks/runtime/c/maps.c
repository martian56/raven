#include <stdio.h>
#include <stdlib.h>
#define CAP (1 << 21)
long keys[CAP], vals[CAP]; char used[CAP];
static unsigned long h(long k) { return (unsigned long)k * 11400714819323198485ul; }
void set(long k, long v) { unsigned long i = h(k) & (CAP - 1); while (used[i] && keys[i] != k) i = (i + 1) & (CAP - 1); used[i] = 1; keys[i] = k; vals[i] = v; }
long get(long k) { unsigned long i = h(k) & (CAP - 1); while (used[i]) { if (keys[i] == k) return vals[i]; i = (i + 1) & (CAP - 1); } return 0; }
int main(void) {
    for (long i = 0; i < 1000000; i++) set(i, i * 2);
    long sum = 0;
    for (long j = 0; j < 1000000; j++) sum += get(j);
    printf("%ld\n", sum);
    return 0;
}
