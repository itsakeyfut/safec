int g(int *q, int n);

int f(int *p, int a[g(p, 1) + (p = 0, 1)]) {
    return 0;
}
