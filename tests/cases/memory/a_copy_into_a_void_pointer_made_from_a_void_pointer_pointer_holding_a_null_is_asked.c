void *memcpy(void *d, void *s, int n); void *malloc(int n); void use(void);
void **f(void) { int i = 1; void **q = malloc(16); if (!q) return 0; *q = 0; void *d = q; void *s = &i; memcpy(d, s, 4); return q; }
