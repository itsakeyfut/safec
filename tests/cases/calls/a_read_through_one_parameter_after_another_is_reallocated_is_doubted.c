void *realloc(void *p, int n);

int f(int *a, int *b) {
    if (a == 0) {
        return 0;
    }
    int *c = realloc(b, 64);
    if (c == 0) {
        return 0;
    }
    return *a + c[0];
}
