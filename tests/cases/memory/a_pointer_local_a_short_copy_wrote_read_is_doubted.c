void *memcpy(void *d, void *s, int n);
int f(int x) { int *p; memcpy(&p, &x, 4); return p == 0; }
