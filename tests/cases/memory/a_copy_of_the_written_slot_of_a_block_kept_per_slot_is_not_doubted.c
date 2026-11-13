void *malloc(int n); void *memcpy(void *d, void *s, int n);
void s(int ** _Nonnull out, int * _Nonnull x) { int **b = malloc(16); if (!b) return; b[0] = x; memcpy(out, b, 8); }
