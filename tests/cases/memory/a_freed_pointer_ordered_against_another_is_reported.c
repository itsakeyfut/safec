void *malloc(int n);
void free(void *p);
int f(int *x) { int *p = malloc(4); if (!p) return 0; free(p); return p < x; }
