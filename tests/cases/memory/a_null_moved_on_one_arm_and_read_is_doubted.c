int f(int c) {
    int a[4];
    a[0] = 0; a[1] = 1; a[2] = 2; a[3] = 3;
    int *p = 0;
    int *q = a;
    if (c) q = p + 1;
    return q[0];
}
