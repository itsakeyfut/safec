void *malloc(int n); void free(void *p);
int f(int **x) {
    int ***box = malloc(16);
    int **b = malloc(16);
    if (!box || !b) return 0;
    if (!x) return 0;
    box[0] = b;
    box[1] = b;
    int **d = *box;
    if (!d) return 0;
    **box = *x;
    int *p = b[1];
    free(b);
    free(box);
    return p == 0;
}
