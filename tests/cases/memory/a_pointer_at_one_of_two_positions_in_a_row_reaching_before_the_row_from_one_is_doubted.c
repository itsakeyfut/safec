int f(int c) {
    int a[3][2];
    a[0][0] = 0; a[0][1] = 1; a[1][0] = 2; a[1][1] = 3; a[2][0] = 4; a[2][1] = 5;
    int *p = a[1];
    int *q;
    if (c) q = p; else q = p - 1;
    return q[0];
}
