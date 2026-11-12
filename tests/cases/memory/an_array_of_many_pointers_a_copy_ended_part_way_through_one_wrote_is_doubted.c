void *memcpy(void *d, void *s, int n);
int f(int **src) { if (!src) return 0; int *big[100]; memcpy(big, src, 12); int *p = big[0]; return p == 0; }
