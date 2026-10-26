int f(void **pp) {
    if (pp == 0) { return 1; }
    int *m[1];
    int **p = m;
    *pp = m;
    void *t = *pp;
    int *u = t;
    if (u == 0) { return 1; }
    *u = 1;
    int *q = *p;
    if (q == 0) { return 1; }
    *q = 1;
    return 0;
}
