void *malloc(int n);
int f(int * _Nonnull a, int n) { int *e = a + n; return e != a; }
