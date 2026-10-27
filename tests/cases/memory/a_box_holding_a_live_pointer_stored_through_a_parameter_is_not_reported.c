void *malloc(int n);

void put(int *** _Nonnull ppp) {
    int **b = malloc(8);
    if (b == 0) {
        *ppp = 0;
        return;
    }
    int *x = malloc(4);
    if (x == 0) {
        *ppp = 0;
        return;
    }
    *x = 1;
    *b = x;
    *ppp = b;
}
