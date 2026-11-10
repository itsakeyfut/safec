void *malloc(int n);
void f(int *** _Nonnull out, int * _Nonnull x) { int **b = malloc(8); int **p = b + 1; *out = p; if (!b) return; *b = x; }
