void *malloc(int n);
void free(void *p);
void put(int ** _Nonnull pp, int c) {
    int *x;
    if (c) {
        x = malloc(4);
    } else {
        x = malloc(8);
    }
    if (x == 0) {
        return;
    }
    *pp = x;
    free(x);
}
