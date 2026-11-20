void *malloc(unsigned long long n);
void free(void *p);
void *memset(void *d, int c, unsigned long long n);
void g(int *q) {
    if (q) {
        *q = 1;
    }
}
int after_free(void) {
    int *p = malloc(4);
    if (!p) {
        return 1;
    }
    int *a[1];
    a[0] = p;
    free(p);
    int *q;
    *(long long *)&q = *(long long *)a;
    g(q);
    return 0;
}
int zeroed_and_unused(void) {
    int x = 0;
    int *p = &x;
    memset(&p, 0, sizeof p);
    return x;
}
int zeroed_then_replaced(int *r) {
    int *p = r;
    memset(&p, 0, sizeof p);
    p = r;
    return p == r;
}
