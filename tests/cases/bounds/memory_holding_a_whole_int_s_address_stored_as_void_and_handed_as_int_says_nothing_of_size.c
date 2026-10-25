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
    int *r = malloc(4);
    if (r == 0) {
        return 1;
    }
    void **m = malloc(8);
    if (m == 0) {
        return 1;
    }
    *m = r;
    void *pm = m;
    use(pm);
    return 0;
}
