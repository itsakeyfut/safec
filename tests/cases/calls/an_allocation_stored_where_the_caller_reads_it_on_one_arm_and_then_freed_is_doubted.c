void *malloc(int n);
void free(void *p);
void put(int ** _Nonnull pp, int c) {
    int *x = malloc(4);
    if (x == 0) {
        return;
    }
    if (c) {
        *pp = x;
    }
    free(x);
}
