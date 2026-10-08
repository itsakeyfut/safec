int f(int *p) {
    int *q;
    int *r;
    if ((r = q = p)) {
        return *p + *q + *r;
    }
    return 0;
}
