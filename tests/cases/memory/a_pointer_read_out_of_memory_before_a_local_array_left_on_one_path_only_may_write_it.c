int f(int ***pp, int ***b, int c) {
    if (pp == 0) { return 1; }
    if (b == 0) { return 1; }
    int **t = 0;
    int *m[1];
    if (c) {
        t = *pp;
        *b = m;
    } else {
        *b = m;
        t = *pp;
    }
    if (t == 0) { return 1; }
    *t = 0;
    int *q = m[0];
    if (q == 0) { return 1; }
    *q = 1;
    return 0;
}
