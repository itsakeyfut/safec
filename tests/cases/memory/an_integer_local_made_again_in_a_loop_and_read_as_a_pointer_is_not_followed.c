int f(void) {
    int i = 0;
    int r = 0;
    while (i < 2) {
        int x;
        void *v = &x;
        int **pp = v;
        int *q = *pp;
        r = q == 0;
        i = i + 1;
    }
    return r;
}
