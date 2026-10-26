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
    char c;
    void **m = malloc(8);
    if (m == 0) {
        return 1;
    }
    *m = &c;
    void *pm = m;
    int **pi = pm;
    int ***b = &pi;
    use(*b);
    return 0;
}
