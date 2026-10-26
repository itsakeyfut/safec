int f(int (*pa)[3]) {
    int a[3];
    int *p = a;
    int *q = *pa;
    return 0;
}
