int f(void) {
    char b[8];
    void *v = b;
    int **q = v;
    int *p = *q;
    return p == 0;
}
