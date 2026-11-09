void *malloc(int n);
void *calloc(int n, int m);
int **f(int * _Nonnull x) { int **b = malloc(16); if (!b) return 0; for (int i = 0; i < 2; i++) b[i] = x; return b; }
