void *memcpy(void *d, void *s, int n);
int f(int x) { char b[8]; memcpy(b, &x, 4); void *v = b; int **q = v; int *p = *q; return p == 0; }
