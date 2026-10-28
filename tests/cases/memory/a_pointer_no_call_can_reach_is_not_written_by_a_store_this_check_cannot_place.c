int f(int ***b, int ****w) {
    int *m[1];
    int *x = 0;
    int **t = &x;
    int ***pt = &t;
    *b = m;
    int ***c = *w;
    *c = *b;
    *t = 0;
    int *q = m[0];
    if (q == 0) { return 1; }
    *q = 1;
    return pt == 0;
}
