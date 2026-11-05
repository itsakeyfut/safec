void *malloc(int n);
void use(int **p) {
    if (!p) return;
    int *q = *p;
    if (!q) return;
    *q = 1;
}
int f(int c) {
    int **box = malloc(8);
    if (!box) return 0;
    void *vb = box;
    int *ib = vb;
    *ib = 7;
    use(box);
    return 0;
}
