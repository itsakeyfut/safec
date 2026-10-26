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
    *m = c;
    void *pm = m;
    use(pm);
    return 0;
}
