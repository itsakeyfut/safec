int f(int a, int **pp) {
    int x;
    int *p;
    x = +a;
    p = &x;
    **pp = x;
    return **pp;
}
