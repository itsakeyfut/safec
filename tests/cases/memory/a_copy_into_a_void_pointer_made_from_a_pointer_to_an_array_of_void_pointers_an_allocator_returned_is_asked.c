void *memcpy(void *d, void *s, int n); void *malloc(int n); void use(void);
void f(void ***pp) { int i = 1; if (!pp) return; void *(*q)[2] = malloc(16); if (!q) return; void *d = q; void *s = &i; memcpy(d, s, 4); use(); }
