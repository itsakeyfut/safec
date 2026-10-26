void run(int *x) {
    void *v = x;
    char **pi = v;
    char *p = *pi;
    if (p != 0) {
        *p = 1;
    }
}
