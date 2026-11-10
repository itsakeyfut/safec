void *malloc(int n);
void **returned(void) {
    int **b = malloc(8);
    void **box = malloc(8);
    if (!box) return 0;
    *box = b;
    return box;
}
