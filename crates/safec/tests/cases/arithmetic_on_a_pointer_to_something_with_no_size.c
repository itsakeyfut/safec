int g(void);
int f(void *v, int *p) {
    void *a = v + 1;
    v += 1;
    int (*fp)(void) = g;
    fp = fp + 1;
    p = p + 1;
    p += 1;
    return 0;
}
