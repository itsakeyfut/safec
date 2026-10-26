void run(int *x) {
    void *v = x;
    int **pi = v;
    int *p = *pi;
    if (p != 0) {
        *p = 1;
    }
}
