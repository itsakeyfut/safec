void *malloc(int n);
int ***returned(int c, int *x) {
    int **b = malloc(8);
    if (!b) return 0;
    int ***box = malloc(8);
    if (!box) return 0;
    if (c) *b = x;
    *box = b;
    return box;
}
