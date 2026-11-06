void *memcpy(void *d, void *s, int n); void use(void);
void f(void **out) { int i = 1; void **q = out + 1; void *d = q; void *s = &i; memcpy(d, s, 4); use(); }
