void *malloc(int n);
void free(void *p);
int b(void) {
    void **q = malloc(8);
    if (!q) return 0;
    void *v = q;
    int *ip = v;
    void **w = v;
    *q = v;
    *ip = 1;
    void *r = *w;
    free(q);
    return r == 0;
}
