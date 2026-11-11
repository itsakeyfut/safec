void *malloc(int n); void *memset(void *d, int c, int n); void use(void);
void e(int ***pp, int *x) { if (!pp) return; int **q = malloc(16); if (!q) return; q[0] = x; q[1] = x; *pp = q; use(); }
