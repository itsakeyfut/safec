void *memcpy(void *d, void *s, int n); void use(void);
void f(void ***ppp) { int i = 1; void *d = *ppp; void *s = &i; memcpy(d, s, 4); use(); }
