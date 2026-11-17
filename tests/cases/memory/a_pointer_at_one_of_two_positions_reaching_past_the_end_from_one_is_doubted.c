int f(int c) {
    int a[4];
    a[0] = 0; a[1] = 1; a[2] = 2; a[3] = 3;
    int *q;
    if (c) q = a + 1; else q = a + 3;
    return q[1];
}
