int g(int n);

int f(int *p, char c) {
    int a[3];
    if (p) {
        return 1;
    }
    if (a) {
        return 2;
    }
    if (g) {
        return 3;
    }
    while (c) {
        return 4;
    }
    return p ? 5 : 6;
}
