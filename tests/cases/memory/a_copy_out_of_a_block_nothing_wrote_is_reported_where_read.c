void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(void) { int **m = malloc(8); if (!m) return 0; int **n = malloc(8); if (!n) return 0; memcpy(n, m, 8); int *r = *n; if (!r) return 0; return *r; }
