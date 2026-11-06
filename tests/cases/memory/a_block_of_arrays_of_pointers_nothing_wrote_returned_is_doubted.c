void *malloc(int n);
int *(*f(void))[2] {
    int *(*b)[2] = malloc(16);
    if (!b) return 0;
    return b;
}
