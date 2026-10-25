int f(int **pp, void **rr, int c) {
    if (pp == 0) {
        return 0;
    }
    if (rr == 0) {
        return 0;
    }
    int *q = *pp;
    if (c) {
        q = *rr;
    }
    if (q == 0) {
        return 0;
    }
    *q = 5;
    return 0;
}
