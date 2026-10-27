void *malloc(int n);
void free(void *p);
void keep(int **where);
void use(void); int main(void) { int *slot = 0; keep(&slot); int *x = malloc(4); if (x == 0) { return 1; } free(x); slot = x; use(); return 0; }
