void *malloc(int n);

int f(int c) {
    int *p = malloc(4);
    if (c) {
        p = malloc(1);
    }
    if (p == 0) {
        return 1;
    }
    *p = 5;
    return 0;
}
