void *malloc(int n);

int *f(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    return p + 5;
}
