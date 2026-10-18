int f(int *s, int c) {
    int *q = s;
    q = s + (q++ && c);
    return 0;
}
