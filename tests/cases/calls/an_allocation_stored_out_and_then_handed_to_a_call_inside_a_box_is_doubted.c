void *malloc(int n);
void free(void *p);
void release_all(int **b);
void put(int ** _Nonnull pp) {
    int *x = malloc(4);
    if (x == 0) {
        return;
    }
    int **b = malloc(8);
    if (b == 0) {
        free(x);
        return;
    }
    *pp = x;
    *b = x;
    release_all(b);
}
