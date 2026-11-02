void *malloc(int n);
void use(int **pi) {
    if (pi != 0) {
        int *p = *pi;
        if (p != 0) {
            *p = 1;
        }
    }
}
void f(char **cc, int k) {
    if (cc == 0) {
        return;
    }
    char *c;
    if (k) {
        c = *cc;
    } else {
        c = 0;
    }
    void **m = malloc(8);
    if (m == 0) {
        return;
    }
    *m = c;
    void *pm = m;
    use(pm);
}
