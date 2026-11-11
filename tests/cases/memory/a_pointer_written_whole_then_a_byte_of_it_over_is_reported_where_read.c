void *malloc(int n);
int r(int *x) { int **m = malloc(8); if (!m) return 0; *m = x; void *v = m; char *c = v; c[3] = 1; int *q = *m; return q != 0; }
