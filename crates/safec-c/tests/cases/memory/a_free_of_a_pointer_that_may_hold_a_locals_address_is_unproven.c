void *malloc(int n);
void free(void *p);

int g(int c) {
    int x;
    int *r = malloc(4);
    if (c) {
        r = &x;
    }
    free(r);
    return 0;
}
