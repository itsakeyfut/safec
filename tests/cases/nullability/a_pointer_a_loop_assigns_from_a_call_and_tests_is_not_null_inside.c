int *next(int *q);
int f(int *q) {
    int n = 0;
    while ((q = next(q))) {
        n = n + *q;
    }
    return n;
}
