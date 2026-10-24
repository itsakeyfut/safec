int f(int *s, int c) {
    int *q = s;
    q = c ? (q = s) : s;
    return 0;
}
