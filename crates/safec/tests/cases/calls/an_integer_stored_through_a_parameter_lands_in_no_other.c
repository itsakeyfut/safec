int f(int a, int *w, int **pp) {
    int x;
    int *p;
    if (!w) return 0;
    if (!pp) return 0;
    x = +a;
    p = &x;
    *w = x;
    int *q = *pp;
    if (!q) return 0;
    return *q;
}
