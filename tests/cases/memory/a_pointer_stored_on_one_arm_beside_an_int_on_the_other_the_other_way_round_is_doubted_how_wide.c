void *malloc(int n);
int f(int c) {
    int **box = malloc(8);
    if (!box) return 0;
    void *vb = box;
    int *ib = vb;
    if (c) { *box = malloc(4); } else { *ib = 7; }
    int *s = *box;
    if (!s) return 0;
    *s = 1;
    return 0;
}
