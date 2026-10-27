void *malloc(int n);
void free(void *p);

void put(int *** _Nonnull ppp) {
    int **b = malloc(8);
    if (b == 0) {
        return;
    }
    int *x = malloc(4);
    if (x == 0) {
        free(b);
        return;
    }
    free(x);
    *b = x;
    *ppp = b;
}
