void *malloc(int n);
int g(int c) {
    void *u = 0;
    void *q = 0;
    int i = 0;
    while (i < 2) {
        u = q;
        q = malloc(1);
        if (!q) return 0;
        i = i + 1;
    }
    int *x = malloc(4);
    if (!x) return 0;
    if (c) { x = u; }
    int **box = malloc(8);
    if (!box) return 0;
    *box = x;
    int *t = *box;
    if (!t) return 0;
    *t = 1;
    return 0;
}
