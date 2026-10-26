void *malloc(int n);

void use(int **pi) {
    if (pi != 0) {
        int *p = *pi;
        if (p != 0) {
            *p = 1;
        }
    }
}

void **make(void) {
    void **m = malloc(8);
    if (m != 0) {
        *m = malloc(1);
    }
    return m;
}

int main(void) {
    void **m = make();
    void *pm = m;
    use(pm);
    return 0;
}
