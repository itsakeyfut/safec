void *malloc(int n);
void free(void *p);
void keep(int ***q);

void d1(void) { int **b = malloc(8); if (b == 0) return; int *x = malloc(4); if (x == 0) { free(b); return; } free(x); *b = x; int **slot = 0; keep(&slot); slot = b; }
