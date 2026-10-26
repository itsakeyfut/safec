void *memcpy(void *s1, void *s2, int n);
int f(int ***pp, int **s) {
    if (pp == 0) { return 1; }
    if (s == 0) { return 1; }
    int **t = *pp;
    if (t == 0) { return 1; }
    int *m[1];
    int **p = m;
    memcpy(t, s, 1);
    int *q = *p;
    if (q == 0) { return 1; }
    *q = 1;
    return 0;
}
