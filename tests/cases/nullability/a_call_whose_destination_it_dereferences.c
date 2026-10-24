int *g(int v);
int f(int *p) {
    p = g(*p);
    *p = 1;
    return 0;
}
