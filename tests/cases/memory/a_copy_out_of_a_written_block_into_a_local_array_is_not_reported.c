void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int *x) { int **m = malloc(8); if (!m) return 0; *m = x; int *a[1]; memcpy(a, m, 8); int *r = a[0]; return r == x; }
