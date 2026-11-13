// The caller may read the slot past the old size, which nothing wrote
// (#918's review).
void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
int **grow(int * _Nonnull x) {
    int **b = malloc(8);
    if (!b) return 0;
    *b = x;
    int **c = realloc(b, 16);
    if (!c) { free(b); return 0; }
    return c;
}
