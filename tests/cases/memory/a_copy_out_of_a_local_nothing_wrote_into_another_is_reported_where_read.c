void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int *x) { int *p; int *q; memcpy(&q, &p, 8); int *r = q; return r == x; }
