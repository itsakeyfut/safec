void *memcpy(void *d, void *s, int n);
int f(int x) { int *m[1]; memcpy(m, &x, 4); int *p = m[0]; return p == 0; }
