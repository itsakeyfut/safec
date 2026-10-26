void *malloc(int n);

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
    int **pi = pm;
    if (pi != 0) {
        int *p = *pi;
        if (p != 0) {
            *p = 1;
        }
    }
    return 0;
}
