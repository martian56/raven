#include <stdio.h>
int main(void) {
    long size = 1600, max_iter = 50, inside = 0;
    for (long y = 0; y < size; y++) {
        double ci = 2.0 * (double)y / (double)size - 1.0;
        for (long x = 0; x < size; x++) {
            double cr = 2.0 * (double)x / (double)size - 1.5;
            double zr = 0.0, zi = 0.0;
            long i = 0;
            while (i < max_iter && zr * zr + zi * zi <= 4.0) {
                double t = zr * zr - zi * zi + cr;
                zi = 2.0 * zr * zi + ci;
                zr = t;
                i++;
            }
            if (i == max_iter) inside++;
        }
    }
    printf("%ld\n", inside);
    return 0;
}
