void *memcpy(void *d, void *s, int n);
int f(void) {
    int x = 1;
    int *z;
    int **pp = &z;
    pp = pp + 1;
    int *px = &x;
    memcpy(pp, &px, 8);
    int *q = z;
    return q != 0;
}
