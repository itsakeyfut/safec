void *malloc(int n);
int *f(int * _Nonnull x, int c) { int **b = malloc(16); if (!b) return 0; int **m = malloc(16); if (!m) return 0; b[0] = x; int **p = c ? b : m; p[1] = x; int *r = b[1]; return r; }
