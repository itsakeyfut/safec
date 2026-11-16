void *malloc(int n); void *memcpy(void *d, void *s, int n); void *memmove(void *d, void *s, int n); char *strcpy(char *d, char *s); void free(void *p);
int f(int *x, int c) { int n = c ? 0 : 8; int **b = malloc(16); if (!b) return 0; memcpy(b, &x, n); int *p = b[0]; free(b); return p == 0; }
