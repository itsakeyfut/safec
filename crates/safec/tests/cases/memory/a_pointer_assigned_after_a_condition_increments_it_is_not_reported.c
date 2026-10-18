int f(int *s) {
    int *q = s;
    q = q++ ? s : s;
    return 0;
}
