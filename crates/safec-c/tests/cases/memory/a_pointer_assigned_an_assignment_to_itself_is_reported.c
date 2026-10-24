int f(int *s) {
    int *q = s;
    q = (q = s);
    return 0;
}
