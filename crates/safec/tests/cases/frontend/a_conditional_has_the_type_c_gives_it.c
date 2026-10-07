int f(int c, int *p) {
    int *r = c ? p : 0;
    return *r;
}
