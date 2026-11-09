void *calloc(int n, int m);
int **f(int * _Nonnull x) { int **b = calloc(2, 8); if (!b) return 0; b[0] = x; return b; }
