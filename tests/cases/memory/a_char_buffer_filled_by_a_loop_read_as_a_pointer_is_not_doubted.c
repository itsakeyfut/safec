int f(void) { char b[8]; for (int i = 0; i < 8; i++) b[i] = 0; void *v = b; int **q = v; int *p = *q; return p == 0; }
