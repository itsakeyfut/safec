void *malloc(int n);
void free(void *p);
void put(int ** _Nonnull pp) {
    int *x = malloc(4);
    if (x == 0) {
        return;
    }
    *pp = x;
    free(x);
    *pp = 0;
}
