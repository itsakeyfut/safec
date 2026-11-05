void *malloc(int n);
int f(void) {
    int ****m = malloc(8); if (!m) return 0;
    int ***n = malloc(8); if (!n) return 0;
    *m = n;
    int *r = ***m;
    return r != 0;
}
