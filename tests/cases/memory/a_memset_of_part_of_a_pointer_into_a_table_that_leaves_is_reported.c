void *malloc(int n); void *memset(void *d, int c, int n); void use(void);
void d(void ***pp) { if (!pp) return; void *m = malloc(16); if (!m) return; memset(m, 1, 4); void **q = m; *pp = q; use(); }
