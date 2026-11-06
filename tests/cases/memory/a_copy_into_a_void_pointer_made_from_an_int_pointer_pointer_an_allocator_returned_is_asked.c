void *memcpy(void *d, void *s, int n); void *malloc(int n); void *realloc(void *p, int n); void use(void);
void f(int ***pp) { int i = 1; if (!pp) return; int **q = malloc(16); if (!q) return; *pp = q; void *d = q; void *s = &i; memcpy(d, s, 4); use(); }
