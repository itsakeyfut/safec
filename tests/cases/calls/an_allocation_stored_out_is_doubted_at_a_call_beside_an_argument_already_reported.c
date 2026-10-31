void *malloc(int n);
void free(void *p);
void rel2(int *a, int *b);
void put(int ** _Nonnull pp, int ** _Nonnull q, int c) {
    int *x = malloc(4);
    if (x == 0) {
        return;
    }
    int *y = malloc(4);
    if (y == 0) {
        return;
    }
    if (c) {
        free(y);
    }
    *pp = x;
    *q = x;
    rel2(y, *q);
}
