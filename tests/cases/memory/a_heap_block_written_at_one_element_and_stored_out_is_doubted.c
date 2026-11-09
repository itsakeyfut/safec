void *malloc(int n);
void *calloc(int n, int m);
void f(int *** _Nonnull out, int * _Nonnull x) { int **b = malloc(16); if (!b) return; b[0] = x; *out = b; }
