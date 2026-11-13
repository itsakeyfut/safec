// Twenty bytes are kept whole, so `b[0] = &x;` may have missed `b[1]`
// (#918's review).
void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
int f(void) {
    int **b = malloc(20);
    if (!b) return 0;
    int x = 0;
    b[0] = &x;
    int *r = b[1];
    free(b);
    return r == 0;
}
