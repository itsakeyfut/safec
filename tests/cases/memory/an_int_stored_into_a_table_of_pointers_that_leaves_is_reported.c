void *malloc(int n); void *memset(void *d, int c, int n); void use(void);
void b(void ***pp) { if (!pp) return; void **q = malloc(16); if (!q) return; void *v = q; int *ip = v; *ip = 1; *pp = q; use(); }
