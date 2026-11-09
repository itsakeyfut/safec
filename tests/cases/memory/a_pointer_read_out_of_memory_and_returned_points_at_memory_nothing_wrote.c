void *malloc(int n);
int **f(void) { int ***box = malloc(8); if (!box) return 0; int **b = malloc(8); *box = b; return *box; }
