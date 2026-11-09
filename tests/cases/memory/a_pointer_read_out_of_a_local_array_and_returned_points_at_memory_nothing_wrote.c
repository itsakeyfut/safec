void *malloc(int n);
int **f(void) { int **b = malloc(8); int **a[2]; a[0] = b; a[1] = b; return a[0]; }
