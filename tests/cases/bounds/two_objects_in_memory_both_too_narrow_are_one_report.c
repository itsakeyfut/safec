void *malloc(int n);

void use(int **pi) {
    if (pi != 0) {
        int *p = *pi;
        if (p != 0) {
            *p = 1;
        }
    }
}

int main(void) {
    char *c = malloc(1);
    if (c == 0) {
        return 1;
    }
    void **m = malloc(8);
    if (m == 0) {
        return 1;
    }
    void **n = malloc(8);
    if (n == 0) {
        return 1;
    }
    *m = c;
    *n = c;
    void *pm = m;
    if (c) {
        pm = n;
    }
    use(pm);
    return 0;
}
