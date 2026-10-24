void h(void);

int f(int x, int *p, int (**pp)(int)) {
    int a[3];
    return x(1) + p(1) + pp(1) + a(1) + h()(1);
}
