void *malloc(int n); void *memset(void *d, int c, int n); void use(void);
int r(void) { int **m = malloc(8); if (!m) return 0; void *v = m; char *c = v; *c = 1; int *q = *m; return q != 0; }
