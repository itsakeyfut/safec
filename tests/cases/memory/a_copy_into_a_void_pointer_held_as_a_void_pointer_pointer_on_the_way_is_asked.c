void *memcpy(void *d, void *s, int n); void *malloc(int n); void *realloc(void *p, int n); void use(void);
void f(void *v) { int i = 1; if (!v) return; void **q = v; void *d = q; void *s = &i; memcpy(d, s, 4); use(); }
