void *memcpy(void *d, void *s, int n);
void f(int **out, int *x, int n) {
    if (!out) return;
    int i = 0;
    while (i < n) {
        char b[8];
        if (i == 1) memcpy(out, b, 8);
        memcpy(b, &x, 8);
        i = i + 1;
    }
}
