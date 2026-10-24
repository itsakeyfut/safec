void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);

int g(int c) {
    int x;
    int *r = malloc(4);
    if (c) {
        r = &x;
    }
    int *q = realloc(r, 8);
    free(q);
    return 0;
}
