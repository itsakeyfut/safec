int f(int ***pp) {
    if (pp == 0) { return 1; }
    int **t = *pp;
    if (t == 0) { return 1; }
    int *m[1];
    int **p = m;
    *t = 0;
    int *q = *p;
    if (q == 0) { return 1; }
    *q = 1;
    return 0;
}
