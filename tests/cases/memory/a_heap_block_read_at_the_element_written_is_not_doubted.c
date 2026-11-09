void *malloc(int n);
int *f(int * _Nonnull x) { int **b = malloc(16); if (!b) return 0; int **c = b; b[0] = x; int *r = c[0]; return r; }
