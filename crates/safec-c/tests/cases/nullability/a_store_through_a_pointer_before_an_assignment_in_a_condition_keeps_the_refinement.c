int f(int *p) {
    int x = 0;
    int *s = &x;
    int *q;
    if ((*s = 1, q = p)) {
        return *q + *p;
    }
    return 0;
}
