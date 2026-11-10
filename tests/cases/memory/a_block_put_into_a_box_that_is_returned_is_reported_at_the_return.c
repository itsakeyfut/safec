void *malloc(int n);
int ***returned(void) {
    int **b = malloc(8);
    int ***box = malloc(8);
    if (!box) return 0;
    *box = b;
    return box;
}
