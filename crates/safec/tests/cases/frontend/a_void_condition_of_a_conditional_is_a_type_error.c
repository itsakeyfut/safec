void h(void);

int f(void) {
    int *q = h() ? 1 : 2;
    return h() ? 1 : 2;
}
