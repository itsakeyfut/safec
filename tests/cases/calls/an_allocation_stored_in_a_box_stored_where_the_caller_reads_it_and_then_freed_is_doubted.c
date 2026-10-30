void *malloc(int n);
void free(void *p);
void put(int *** _Nonnull pp) {
    int *x = malloc(4);
    if (x == 0) {
        return;
    }
    int **b = malloc(8);
    if (b == 0) {
        free(x);
        return;
    }
    *b = x;
    *pp = b;
    free(x);
}
