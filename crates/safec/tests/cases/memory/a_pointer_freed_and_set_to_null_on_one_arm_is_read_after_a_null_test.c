void *malloc(int n);
void free(void *p);

int f(int c) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    if (c) {
        free(p);
        p = 0;
    }
    if (p != 0) {
        return *p;
    }
    return 0;
}
