void *malloc(int n); void *memcpy(void *d, void *s, int n);
int f(int *x) { int *p; int **pp = &p; int ***ppp = &pp; int **n = malloc(8); if (!n) return 0; memcpy(n, *ppp, 8); int *r = *n; return r == x; }
