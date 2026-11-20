int f(void) {
    int x;
    {
        int *a;
        a = &x;
        goto M;
    L:
        return a != 0;
    }
M:
    goto L;
}
