#include <stdio.h>
#include <math.h>
typedef struct { double x, y, z, vx, vy, vz, m; } Body;
static void advance(Body *b, int n, double dt) {
    for (int i = 0; i < n; i++) {
        Body *a = &b[i];
        for (int j = i + 1; j < n; j++) {
            Body *c = &b[j];
            double dx = a->x - c->x, dy = a->y - c->y, dz = a->z - c->z;
            double d2 = dx * dx + dy * dy + dz * dz;
            double mag = dt / (d2 * sqrt(d2));
            a->vx -= dx * c->m * mag; a->vy -= dy * c->m * mag; a->vz -= dz * c->m * mag;
            c->vx += dx * a->m * mag; c->vy += dy * a->m * mag; c->vz += dz * a->m * mag;
        }
    }
    for (int i = 0; i < n; i++) { b[i].x += dt * b[i].vx; b[i].y += dt * b[i].vy; b[i].z += dt * b[i].vz; }
}
static double energy(Body *b, int n) {
    double e = 0;
    for (int i = 0; i < n; i++) {
        e += 0.5 * b[i].m * (b[i].vx * b[i].vx + b[i].vy * b[i].vy + b[i].vz * b[i].vz);
        for (int j = i + 1; j < n; j++) {
            double dx = b[i].x - b[j].x, dy = b[i].y - b[j].y, dz = b[i].z - b[j].z;
            e -= b[i].m * b[j].m / sqrt(dx * dx + dy * dy + dz * dz);
        }
    }
    return e;
}
int main(void) {
    Body b[5] = {{0,0,0,0,0,0,39.47},{4.84,-1.16,-0.10,0.60,2.81,-0.02,0.037},{8.34,4.12,-0.40,-1.01,1.82,0.008,0.011},{12.89,-15.11,-0.22,1.08,0.86,-0.01,0.0017},{15.37,-25.91,0.17,0.97,0.59,-0.03,0.002}};
    printf("%.9f\n", energy(b, 5));
    for (int k = 0; k < 5000000; k++) advance(b, 5, 0.01);
    printf("%.9f\n", energy(b, 5));
    return 0;
}
