void *malloc(int n); void *memset(void *d, int c, int n); void use(void);
void g(int **pp) { if (!pp) return; int *m = malloc(16); if (!m) return; *m = 1; *pp = m; use(); }
