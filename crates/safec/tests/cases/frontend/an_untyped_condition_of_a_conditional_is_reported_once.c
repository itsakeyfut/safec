void h(void);

int f(void) {
    int i = (h() + 1) ? 1 : 2;
    return i;
}
