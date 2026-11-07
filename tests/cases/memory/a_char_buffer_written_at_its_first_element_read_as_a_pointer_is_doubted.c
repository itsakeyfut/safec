int f(void) { char b[8]; b[0] = 0; void *v = b; int **q = v; int *p = *q; return p == 0; }
