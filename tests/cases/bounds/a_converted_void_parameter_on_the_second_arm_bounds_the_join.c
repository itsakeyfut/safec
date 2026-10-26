void run(void **m, int c) {
    int x = 0;
    int *l = &x;
    int **pi;
    void *pm = m;
    if (c) {
        pi = &l;
    } else {
        pi = pm;
    }
    int *p = *pi;
    if (p != 0) {
        *p = 1;
    }
}
