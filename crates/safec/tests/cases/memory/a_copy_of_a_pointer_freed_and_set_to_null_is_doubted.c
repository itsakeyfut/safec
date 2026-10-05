void *malloc(int n);
void free(void *p);

int *f(int c) {
    int *p = malloc(4);
    int *q = p;
    if (c) {
        free(p);
        p = 0;
    }
    return q;
}
