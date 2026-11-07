void exit(int status);

int f(int n, int *q) {
    int *p;
    int ok;
    if (n > 0) {
        p = q;
    }
    ok = n > 0 ? 1 : (exit(2), 0);
    return p != 0 && ok;
}
