void *malloc(int n);
void release_all(int **b);
void put(int ** _Nonnull pp) {
    int *x = malloc(4);
    if (x == 0) {
        return;
    }
    int *a[1];
    a[0] = x;
    *pp = x;
    release_all(a);
}
