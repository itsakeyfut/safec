// A block of 65 pointers is kept whole, so `b[0] = &x;` may have missed
// `b[5]`, which a counted position proves inside the block (#918's review).
void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
int f(void) {
    int **b = malloc(520);
    if (!b) return 0;
    int x = 0;
    b[0] = &x;
    int *r = b[5];
    free(b);
    return r == 0;
}
