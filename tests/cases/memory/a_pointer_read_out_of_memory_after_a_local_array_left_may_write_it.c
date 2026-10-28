int f(int ***pp, int ***b) {
    if (pp == 0) { return 1; }
    if (b == 0) { return 1; }
    int *m[1];
    *b = m;
    int **t = *pp;
    if (t == 0) { return 1; }
    *t = 0;
    int *q = m[0];
    if (q == 0) { return 1; }
    *q = 1;
    return 0;
}
