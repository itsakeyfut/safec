void free(void *p);
void *malloc(int n);

int f(int c) {
    void *p = malloc(4);
    if (!p) {
        return 0;
    }
    free(p);
    (*p, c);
    return 0;
}
