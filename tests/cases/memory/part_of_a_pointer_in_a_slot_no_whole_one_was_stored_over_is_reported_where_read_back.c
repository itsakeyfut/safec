void *malloc(int n);
void free(void *p);
int b(void) {
    void **q = malloc(16);
    if (!q) return 0;
    void *v = q;
    int *ip = v;
    void **w = v;
    *ip = 1;
    q[1] = v;
    void *r = *w;
    free(q);
    return r == 0;
}
