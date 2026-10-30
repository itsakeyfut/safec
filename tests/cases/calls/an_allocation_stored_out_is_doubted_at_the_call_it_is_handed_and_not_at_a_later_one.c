void *malloc(int n);
void keep(int *p);
void tick(int *n);
void put(int ** _Nonnull pp) {
    int *x = malloc(4);
    if (x == 0) {
        return;
    }
    *pp = x;
    keep(x);
    int n = 0;
    tick(&n);
}
