void *memcpy(void *d, void *s, int n);
int f(int *x) {
    int *p;
    int *a[2];
    a[1] = x;
    memcpy(a, &p, 8);
    int *r = a[1];
    return r == x;
}
