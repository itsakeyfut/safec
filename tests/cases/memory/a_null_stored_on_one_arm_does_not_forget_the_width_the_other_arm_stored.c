void *malloc(int n);
int f(int c) {
    int **box = malloc(8);
    if (!box) return 0;
    if (c) { *box = 0; } else { *box = malloc(4); }
    int *s = *box;
    if (!s) return 0;
    *s = 1;
    return 0;
}
