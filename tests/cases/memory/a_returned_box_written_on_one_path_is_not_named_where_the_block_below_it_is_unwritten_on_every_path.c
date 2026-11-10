void *malloc(int n);
int ***returned(int c) {
    int **b = malloc(8);
    int ***box = malloc(8);
    if (!box) return 0;
    if (c) *box = b;
    return box;
}
