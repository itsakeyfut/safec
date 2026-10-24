void *malloc(int n);
void free(void *p);

int f(int *q) {
    int *p = malloc(4);
    if (p == 0 || q == 0) {
        free(p);
        return 0;
    }
    *q = 1;
    free(p);
    return 0;
}
