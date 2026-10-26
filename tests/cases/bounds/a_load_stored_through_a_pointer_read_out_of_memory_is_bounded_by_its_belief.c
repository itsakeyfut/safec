void *malloc(int n);
void use(int **pi) {
    if (pi != 0) {
        int *p = *pi;
        if (p != 0) {
            *p = 1;
        }
    }
}
void f(char ***ccc) {
    if (ccc == 0) {
        return;
    }
    char **cc = *ccc;
    if (cc == 0) {
        return;
    }
    void ***mm = malloc(8);
    if (mm == 0) {
        return;
    }
    void **m = malloc(8);
    if (m == 0) {
        return;
    }
    *mm = m;
    void **t = *mm;
    if (t == 0) {
        return;
    }
    char *c = *cc;
    *t = c;
    void *pm = m;
    use(pm);
}
