#include <stdio.h>
#include <stdlib.h>
typedef struct T { struct T *l, *r; } T;
T *make(int d) {
    T *t = malloc(sizeof(T));
    if (d == 0) { t->l = t->r = 0; return t; }
    t->l = make(d - 1); t->r = make(d - 1); return t;
}
long check(T *t) { return t->l ? 1 + check(t->l) + check(t->r) : 1; }
void freet(T *t) { if (t->l) { freet(t->l); freet(t->r); } free(t); }
int main(void) {
    long total = 0;
    for (int i = 0; i < 40; i++) { T *t = make(16); total += check(t); freet(t); }
    printf("%ld\n", total);
    return 0;
}
