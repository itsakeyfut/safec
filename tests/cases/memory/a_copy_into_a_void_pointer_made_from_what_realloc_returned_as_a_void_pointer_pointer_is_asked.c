void *memcpy(void *d, void *s, int n); void *malloc(int n); void *realloc(void *p, int n); void use(void);
void f(void **out) { int i = 1; void **q = realloc(out, 16); if (!q) return; void *d = q; void *s = &i; memcpy(d, s, 4); use(); }
