void *malloc(int n);

int main(void) {
    int *a = malloc(16);
    if (a == 0) {
        return 1;
    }
    void *v = malloc(8);
    if (v == 0) {
        return 1;
    }
    void **as_void = v;
    int **slot = v;
    *as_void = a + 5;
    int *q = *slot;
    if (q == 0) {
        return 1;
    }
    *q = 1;
    return 0;
}
