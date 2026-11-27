int pointers(void) {
    int x = 4;
    int *ps[2] = {&x};
    return ps[1] == 0;
}

int numbers(void) {
    int a[4] = {1};
    return a[0] + a[3];
}

int past(void) {
    int a[4] = {1, 2};
    return a[4];
}
