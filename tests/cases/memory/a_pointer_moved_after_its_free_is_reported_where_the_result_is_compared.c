void *malloc(int n);
void free(void *p);
int f(void) { int *p = malloc(8); if (!p) return 0; free(p); int *q = p + 1; return q == 0; }
