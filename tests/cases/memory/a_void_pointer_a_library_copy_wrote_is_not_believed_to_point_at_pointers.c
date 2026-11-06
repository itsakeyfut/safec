void *memcpy(void *d, void *s, int n); void use(void);
void f(void **pp) { void *slot = 0; if (!pp) return; memcpy(&slot, pp, 8); if (!slot) return; int i = 1; void *s = &i; memcpy(slot, s, 4); use(); }
