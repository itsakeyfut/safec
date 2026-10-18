int f(int *s, int *r) {
    int *q = s;
    q = &r[!q++];
    return 0;
}
