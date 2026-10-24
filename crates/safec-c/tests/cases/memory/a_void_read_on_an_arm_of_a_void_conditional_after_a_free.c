void free(void *p);
void *malloc(int n);

int f(int c) {
    void *p = malloc(4);
    void *q = malloc(4);
    if (!p) {
        return 0;
    }
    if (!q) {
        return 0;
    }
    free(p);
    c ? *p : *q;
    free(q);
    return 0;
}
