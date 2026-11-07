void *memset(void *d, int c, int n);
int f(void) { int *p; memset(&p, 0, 0); int *r = p; return r == 0; }
