void *malloc(int n);
int r(void) { int **m = malloc(8); if (!m) return 0; void *v = m; char *c = v; c[0] = 1; c[1] = 1; c[2] = 1; c[3] = 1; c[4] = 1; c[5] = 1; c[6] = 1; c[7] = 1; int *q = *m; return q != 0; }
