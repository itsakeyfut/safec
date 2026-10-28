void *malloc(int n);
void free(void *p);

void put(int **** _Nonnull pppp) {
    int ***bb = malloc(8);
    if (bb == 0) {
        *pppp = 0;
        return;
    }
    int **b = malloc(8);
    if (b == 0) {
        free(bb);
        *pppp = 0;
        return;
    }
    int *x = malloc(4);
    if (x == 0) {
        free(b);
        free(bb);
        *pppp = 0;
        return;
    }
    free(x);
    *b = x;
    *bb = b;
    *pppp = bb;
}
