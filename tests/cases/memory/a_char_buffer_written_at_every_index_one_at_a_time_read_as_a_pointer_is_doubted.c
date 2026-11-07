int f(void) { char b[8]; b[0] = 0; b[1] = 0; b[2] = 0; b[3] = 0; b[4] = 0; b[5] = 0; b[6] = 0; b[7] = 0; void *v = b; int **q = v; int *p = *q; return p == 0; }
