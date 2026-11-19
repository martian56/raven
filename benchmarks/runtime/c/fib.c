#include <stdio.h>
long fib(long n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
int main(void) { printf("%ld\n", fib(35)); return 0; }
