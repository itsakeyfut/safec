int *g(int *p);
int f(int *s) {
    int *q = s;
    q = g(q++);
    return 0;
}
