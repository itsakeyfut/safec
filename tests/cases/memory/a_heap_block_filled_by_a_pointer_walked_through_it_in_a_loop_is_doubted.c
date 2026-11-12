void *malloc(int n);
int **f(int * _Nonnull x) { int **b = malloc(16); if (!b) return 0; int **p = b; for (int i = 0; i < 2; i++) { *p = x; p = p + 1; } return b; }
