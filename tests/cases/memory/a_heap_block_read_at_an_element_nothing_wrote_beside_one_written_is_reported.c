void *malloc(int n);
void *calloc(int n, int m);
int *f(int * _Nonnull x) { int **b = malloc(16); if (!b) return 0; b[0] = x; int *r = b[1]; return r; }
