void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int *x, int c) { int **m = malloc(8); if (!m) return 0; int **k = malloc(8); if (!k) return 0; int *a[1]; a[0] = x; void *d = a; if (c) d = k; memcpy(d, m, 8); int *r = a[0]; return r == x; }
