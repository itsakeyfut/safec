int f(int *(*pa)[2]) {
    if (pa == 0) {
        return 0;
    }
    int *q = (*pa)[0];
    if (q != 0) {
        *q = 1;
    }
    return 0;
}
