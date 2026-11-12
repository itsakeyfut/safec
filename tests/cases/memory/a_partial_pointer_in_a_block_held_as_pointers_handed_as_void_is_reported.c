void *malloc(int n); void *memcpy(void *d, void *s, int n); void g(void *p); void use(void *p);
void f(void) { int i = 1; void *m = malloc(8); if (!m) return; memcpy(m, &i, 4); void **h = m; g(m); }
