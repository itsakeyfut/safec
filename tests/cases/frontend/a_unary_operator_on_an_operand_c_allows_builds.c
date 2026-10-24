int g(int n);

int f(int n, char c, int *p) {
    int a[3];
    int i;
    i = -n;
    i = ~n;
    i = !n;
    i = +c;
    i = -c;
    i = ~c;
    i = !c;
    i = !p;
    i = !a;
    i = !g;
    return i;
}
