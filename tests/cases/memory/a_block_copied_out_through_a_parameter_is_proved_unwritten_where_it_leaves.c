void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(void **out) { int **b = malloc(8); if (!b) return; void *v = b; if (!out) return; memcpy(out, &v, 8); }
