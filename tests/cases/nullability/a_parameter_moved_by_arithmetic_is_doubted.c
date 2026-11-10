void *malloc(int n);
int f(int *a, int n) { int *e = a + n; return e != a; }
