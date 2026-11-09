void *malloc(int n);
int **f(int * _Nonnull x) { int **b = malloc(520); if (!b) return 0; b[0] = x; return b; }
