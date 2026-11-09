void *malloc(int n);
void *calloc(int n, int m);
int **f(int * _Nonnull x) { int **b = malloc(16); if (!b) return 0; b[0] = x; b[1] = x; return b; }
