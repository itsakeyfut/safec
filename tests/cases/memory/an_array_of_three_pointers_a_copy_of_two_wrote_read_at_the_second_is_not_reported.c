void *memcpy(void *d, void *s, int n);
int f(int **src) { if (!src) return 0; int *m[3]; memcpy(m, src, 16); int *p = m[1]; return p == 0; }
