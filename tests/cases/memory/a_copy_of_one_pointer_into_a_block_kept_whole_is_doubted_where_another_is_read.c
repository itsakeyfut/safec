// A block of 65 pointers is kept whole, and a copy of one pointer into it
// may have missed the rest (#925).
void *memcpy(void *d, void *s, int n); void *malloc(int n); void free(void *p);
int f(int *x) {
    int **b = malloc(520);
    if (!b) return 0;
    memcpy(b, &x, 8);
    int *r = b[1];
    free(b);
    return r == x;
}
