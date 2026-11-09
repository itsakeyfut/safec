void *malloc(int n);
int *f(int * _Nonnull x, int c) { int **b = malloc(16); if (!b) return 0; b[0] = x; int **p = b; if (c) p = b + 1; int *r = p[0]; return r; }
