int f(int *p, int a[p ? 1 : 2], int b[p ? 2 : 3], int c[(p = 0, 1)]) {
    return 0;
}
