void *malloc(int n); void free(void *p); void *memset(void *d, int c, int n);
void f(void) { int **box = malloc(8); if (!box) return; memset(box, 1, 8); int *t = *box; if (t) *t = 1; free(box); }
