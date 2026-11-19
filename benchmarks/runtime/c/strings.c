#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(void) {
    int n = 200000; char **parts = malloc(sizeof(char *) * n); long total = 0;
    for (int i = 0; i < n; i++) { char buf[32]; int len = sprintf(buf, "item %d", i); parts[i] = malloc(len + 1); memcpy(parts[i], buf, len + 1); }
    for (int i = 0; i < n; i++) total += strlen(parts[i]);
    printf("%ld\n", total);
    return 0;
}
