void *malloc(int n);

void use(int **pi) {
    if (pi != 0) {
        int *p = *pi;
        if (p != 0) {
            *p = 1;
        }
    }
}

void f(char **cc) {
    if (cc == 0) {
        return;
    }
    char *c = *cc;
    void **m = malloc(8);
    if (m == 0) {
        return;
    }
    *m = c;
    void *pm = m;
    use(pm);
}

int main(void) {
    char c;
    char *pc = &c;
    f(&pc);
    return 0;
}
