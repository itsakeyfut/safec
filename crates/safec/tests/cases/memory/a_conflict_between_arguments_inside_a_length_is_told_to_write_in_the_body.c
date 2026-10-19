int g(int *q, int r);

int f(int *p, int a[g(p, (p = 0, 1))]) {
    return 0;
}
