void *memcpy(void *d, void *s, int n); void *malloc(int n); void *realloc(void *p, int n); void use(void);
void *f(void) { int i = 1; void *d = malloc(16); if (!d) return 0; void *s = &i; memcpy(d, s, 4); return d; }
