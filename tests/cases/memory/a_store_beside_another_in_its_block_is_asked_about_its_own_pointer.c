void *malloc(int n);
void g(int ***o1, int ***o2, int *x, int c) {
    int **a = malloc(8);
    int **b = malloc(8);
    if (!o1) return;
    if (!o2) return;
    if (!a) return;
    *a = x;
    *o1 = a; *o2 = b;
    if (c) { if (b) *b = x; }
}
