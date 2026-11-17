int f(int c) {
    int a[2][2];
    a[0][0] = 0; a[0][1] = 1; a[1][0] = 2; a[1][1] = 3;
    int *p = a[0];
    int *q;
    if (c) q = p; else q = p + 1;
    return q[1];
}
