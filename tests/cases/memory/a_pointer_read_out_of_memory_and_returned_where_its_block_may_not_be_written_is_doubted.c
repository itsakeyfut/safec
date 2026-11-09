void *malloc(int n);
int **f(int c, int * _Nonnull x) { int ***box = malloc(8); if (!box) return 0; int **b = malloc(8); if (!b) return 0; if (c) *b = x; *box = b; return *box; }
