void *malloc(int n);
int f(void) {
    int *p = malloc(4);
    *p = 42;
    return 0;
}
