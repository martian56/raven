#include <stdio.h>
#include <stdlib.h>
#include <math.h>
static double a(long i, long j) { return 1.0 / (double)((i + j) * (i + j + 1) / 2 + i + 1); }
static void mul_av(long n, double *v, double *out) {
    for (long i = 0; i < n; i++) { double s = 0; for (long j = 0; j < n; j++) s += a(i, j) * v[j]; out[i] = s; }
}
static void mul_atv(long n, double *v, double *out) {
    for (long i = 0; i < n; i++) { double s = 0; for (long j = 0; j < n; j++) s += a(j, i) * v[j]; out[i] = s; }
}
static void mul_atav(long n, double *v, double *out, double *tmp) { mul_av(n, v, tmp); mul_atv(n, tmp, out); }
int main(void) {
    long n = 2000;
    double *u = malloc(n * sizeof(double)), *v = malloc(n * sizeof(double)), *tmp = malloc(n * sizeof(double));
    for (long i = 0; i < n; i++) { u[i] = 1.0; v[i] = 0.0; }
    for (int k = 0; k < 10; k++) { mul_atav(n, u, v, tmp); mul_atav(n, v, u, tmp); }
    double vbv = 0, vv = 0;
    for (long i = 0; i < n; i++) { vbv += u[i] * v[i]; vv += v[i] * v[i]; }
    printf("%.9f\n", sqrt(vbv / vv));
    return 0;
}
