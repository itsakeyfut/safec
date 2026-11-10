void *malloc(int n);
void *returned(int c) {
    void *e = malloc(8);
    int **d = malloc(8);
    if (!d) return 0;
    *d = e;
    int ****p = malloc(8);
    if (!p) return 0;
    void *vp = p;
    void **pv = vp;
    *pv = d;
    void *x;
    if (c) x = p; else x = d;
    return x;
}
