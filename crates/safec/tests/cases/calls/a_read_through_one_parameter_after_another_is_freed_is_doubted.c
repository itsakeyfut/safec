void free(void *p);

int f(int *a, int *b) {
    if (a == 0) {
        return 0;
    }
    free(b);
    return *a;
}
