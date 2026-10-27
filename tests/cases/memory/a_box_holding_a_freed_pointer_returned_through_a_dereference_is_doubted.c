void *malloc(int n);
void free(void *p);

int **e1(void) { int ***t = malloc(8); if (t == 0) return 0; int **b = malloc(8); if (b == 0) { free(t); return 0; } int *x = malloc(4); if (x == 0) { free(b); free(t); return 0; } free(x); *b = x; *t = b; return *t; }
