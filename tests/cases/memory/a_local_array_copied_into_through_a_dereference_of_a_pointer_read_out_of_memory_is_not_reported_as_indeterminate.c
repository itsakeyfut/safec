void *memcpy(void *s1, void *s2, int n);
int f(int ***out, int ***in, int **s) {
    if (out == 0) { return 1; }
    if (in == 0) { return 1; }
    if (s == 0) { return 1; }
    int *m[1];
    *out = m;
    int **t = *in;
    if (t == 0) { return 1; }
    int ***pp = &t;
    memcpy(*pp, s, 8);
    int **p = m;
    int *q = *p;
    if (q == 0) { return 1; }
    *q = 1;
    *out = 0;
    return 0;
}
