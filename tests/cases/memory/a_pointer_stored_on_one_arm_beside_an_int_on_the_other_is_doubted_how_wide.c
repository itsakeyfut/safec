void *malloc(int n);
int f(int c) {
    int **box = malloc(8);
    if (!box) return 0;
    void *vb = box;
    int *ib = vb;
    if (c) { *ib = 7; } else { *box = malloc(4); }
    int *s = *box;
    if (!s) return 0;
    *s = 1;
    return 0;
}
