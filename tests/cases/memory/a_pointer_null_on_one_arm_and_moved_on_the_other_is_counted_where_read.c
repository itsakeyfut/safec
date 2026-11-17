int f(int c) {
    int a[4];
    a[0] = 0; a[1] = 1; a[2] = 2; a[3] = 3;
    int *q = 0;
    if (c) q = a + 1;
    if (!q) return 0;
    return q[1];
}
