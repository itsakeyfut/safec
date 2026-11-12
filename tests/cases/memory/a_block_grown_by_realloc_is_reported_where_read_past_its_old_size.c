// Bytes past the old size are indeterminate (C17 7.22.3.5 p2), so `c[1]`
// reads a pointer nothing wrote (#918's review).
void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
int f(void) {
    int **b = malloc(8);
    if (!b) return 0;
    int x = 0;
    *b = &x;
    int **c = realloc(b, 16);
    if (!c) { free(b); return 0; }
    int *r = c[1];
    free(c);
    return r == 0;
}
