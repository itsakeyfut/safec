int f(int *(*pa)[2][3]) {
    if (pa == 0) {
        return 0;
    }
    int *q = (*pa)[1][2];
    if (q != 0) {
        *q = 1;
    }
    return 0;
}
