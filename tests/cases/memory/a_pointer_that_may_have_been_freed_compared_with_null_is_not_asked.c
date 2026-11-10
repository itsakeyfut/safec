void *malloc(int n);
void free(void *p);
int f(int c) { int *p = malloc(4); if (!p) return 0; if (c) free(p); return p != 0; }
