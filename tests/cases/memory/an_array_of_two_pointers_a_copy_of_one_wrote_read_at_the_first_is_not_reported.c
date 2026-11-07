void *memcpy(void *d, void *s, int n);
int f(int **src) { if (!src) return 0; int *m[2]; memcpy(m, src, 8); int *p = m[0]; return p == 0; }
