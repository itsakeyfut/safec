void keep(int ***s, int ****into);
int f(int c) {
    int **slot;
    int *m[1];
    int ***box[1];
    keep(&slot, box);
    int ***x = box[0];
    if (x == 0) { return 1; }
    int **t = 0;
    if (c) {
        t = *x;
        slot = m;
    } else {
        slot = m;
        t = *x;
    }
    if (t == 0) { return 1; }
    *t = 0;
    int *q = m[0];
    if (q == 0) { return 1; }
    *q = 1;
    return 0;
}
