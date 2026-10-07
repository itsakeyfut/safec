int g(int n);

int f(int *p, char c) {
    int a[3];
    int i;
    if (p) {
        return 1;
    }
    if (a) {
        return 2;
    }
    if (g) {
        return 3;
    }
    if (c) {
        return 4;
    }
    while (p) {
        return 5;
    }
    while (a) {
        return 6;
    }
    while (g) {
        return 7;
    }
    while (c) {
        return 8;
    }
    for (; p;) {
        return 9;
    }
    for (; a;) {
        return 10;
    }
    for (; g;) {
        return 11;
    }
    for (; c;) {
        return 12;
    }
    i = p ? 13 : 14;
    i = a ? 15 : 16;
    i = g ? 17 : 18;
    return c ? i : 19;
}
