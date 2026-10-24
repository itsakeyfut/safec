void *malloc(int n);
void free(void *p);

int g(int c) {
    int x;
    int *p = malloc(4);
    int *r = p;
    if (c) {
        r = &x;
    }
    free(r);
    free(p);
    return 0;
}
