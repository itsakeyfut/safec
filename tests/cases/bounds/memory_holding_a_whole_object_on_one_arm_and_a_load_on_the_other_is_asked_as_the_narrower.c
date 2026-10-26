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
    char *c = *cc;
    int *wide = malloc(4);
    void **m = malloc(8);
    if (m == 0) {
        return;
    }
    if (k) {
        *m = c;
    } else {
        *m = wide;
    }
    void *pm = m;
    use(pm);
}

int main(void) {
    char c;
    char *pc = &c;
    f(&pc, 1);
    return 0;
}
