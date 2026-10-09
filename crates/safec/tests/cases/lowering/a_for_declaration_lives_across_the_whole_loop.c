int f(int n) {
    int s = 0;
    for (int i = 0, j = n; i < j; i = i + 1) {
        int t = i;
        s = s + t;
    }
    return s;
}

int g(void) {
    for (int x = 0;;) {
        return x;
    }
}
