void *malloc(int n);
void release_pp(int **pp);
void put(int ** _Nonnull pp) {
    int *x = malloc(4);
    if (x == 0) {
        return;
    }
    *pp = x;
    release_pp(&x);
}
