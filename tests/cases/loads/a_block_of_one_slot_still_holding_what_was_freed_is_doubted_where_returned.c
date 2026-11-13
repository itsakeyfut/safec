// #656's program without the overwrite: `b` still holds the freed `x`.
void *malloc(int n);
void free(void *p);
int **make(void) {
    int **b = malloc(8);
    if (b == 0) { return 0; }
    int *x = malloc(4);
    *b = x;
    free(x);
    return b;
}
