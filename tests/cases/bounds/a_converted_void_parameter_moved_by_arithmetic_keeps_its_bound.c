void run(void **m) {
    void *pm = m;
    int **pi = pm;
    int **pj = pi + 1;
    int *p = *pj;
    if (p != 0) {
        *p = 1;
    }
}
