void *malloc(int n);
void free(void *p);

void put(int *** _Nonnull ppp) {
    int **b = malloc(8);
    if (b == 0) {
        return;
    }
    *ppp = b;
    int *x = malloc(4);
    if (x == 0) {
        return;
    }
    free(x);
    *b = x;
}
