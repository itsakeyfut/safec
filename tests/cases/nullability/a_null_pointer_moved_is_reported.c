void *malloc(int n);
int f(void) { int *p = 0; int *q = p + 1; return q != 0; }
