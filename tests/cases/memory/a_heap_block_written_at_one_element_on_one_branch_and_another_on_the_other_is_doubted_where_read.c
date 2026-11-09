void *malloc(int n);
int *f(int * _Nonnull x, int c) { int **b = malloc(16); if (!b) return 0; if (c) b[0] = x; else b[1] = x; int *r = b[0]; return r; }
