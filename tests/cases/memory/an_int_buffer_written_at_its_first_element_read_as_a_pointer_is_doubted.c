int f(void) { int b[4]; b[0] = 0; void *v = b; int **q = v; int *p = *q; return p == 0; }
