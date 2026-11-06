void *memcpy(void *d, void *s, int n); void use(void); void **get(void);
void f(void) { int i = 1; void *d = get(); void *s = &i; memcpy(d, s, 4); use(); }
