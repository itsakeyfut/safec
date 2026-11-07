void *memcpy(void *d, void *s, int n);
int f(int **src) { if (!src) return 0; char b[8]; memcpy(b, src, 8); void *v = b; int **q = v; int *p = *q; return p == 0; }
