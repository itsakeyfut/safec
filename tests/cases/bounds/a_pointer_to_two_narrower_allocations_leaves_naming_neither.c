void *malloc(int n);
void g(int *q);

int f(int c) {
    void *v = malloc(1);
    if (c) {
        v = malloc(2);
    }
    g(v);
    return 0;
}
