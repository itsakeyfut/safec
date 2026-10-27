void *malloc(int n);
void free(void *p);
void put(int *** _Nonnull ppp) { int **pp = *ppp; int *x = malloc(4); if (x == 0) { return; } free(x); *pp = x; }
