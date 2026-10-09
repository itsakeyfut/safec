int f(int *p, int **q) {
    if (q) {
        return *p && **q;
    }
    return 0;
}
