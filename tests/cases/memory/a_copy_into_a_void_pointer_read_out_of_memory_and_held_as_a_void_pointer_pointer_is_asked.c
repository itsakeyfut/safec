void *memcpy(void *d, void *s, int n); void *malloc(int n); void *realloc(void *p, int n); void use(void);
void f(void **pv) { int i = 1; if (!pv) return; void **q = *pv; if (!q) return; void *d = q; void *s = &i; memcpy(d, s, 4); use(); }
