void *malloc(int n);
void f(char **cc, int **ii) {
    if (cc == 0 || ii == 0) {
        return;
    }
    char *c = *cc;
    int *d = *ii;
    void **m = malloc(8);
    if (m == 0) {
        return;
    }
    *m = c;
    *m = d;
    void *pm = m;
    int **pi = pm;
    int *p = *pi;
    if (p != 0) {
        *p = 1;
    }
}
