void *malloc(int n);
void *buffer(int n) {
    void *p = malloc(n);
    return p;
}
