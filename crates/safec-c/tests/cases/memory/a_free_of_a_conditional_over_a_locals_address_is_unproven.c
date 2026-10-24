void *malloc(int n);
void free(void *p);

int g(int c) {
    int x;
    void *v = malloc(4);
    int *p = malloc(4);
    free(c ? &x : v);
    free(c ? &x : p);
    return 0;
}
