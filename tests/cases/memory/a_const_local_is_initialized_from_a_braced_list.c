int initialized(void) {
    const int a[16] = {1};
    const struct {
        int *p;
        int n;
    } q = {0, 2};
    return a[0] + q.n;
}

int written(void) {
    const int a[2] = {1, 2};
    ((int *)a)[0] = 3;
    return a[0];
}
