void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int *x) { int *a[1]; int **n = malloc(8); if (!n) return 0; memcpy(n, a, 8); int *r = *n; return r == x; }
